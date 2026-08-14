//! Pure-Rust OCR via `ort` (ONNX Runtime library — not a subprocess).
//!
//! Loads ddddocr's quantized CRNN model, preprocesses the PNG, runs
//! inference, and CTC-decodes the output to text.
//!
//! `ort` is a Rust wrapper around Microsoft's onnxruntime C library. The
//! dynamic library is auto-downloaded at build time and loaded at first use.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use image::imageops::FilterType;
use ndarray::Array2;
use ort::session::Session;
use ort::value::{Tensor, TensorRef, TensorValueType};
use tracing::warn;

use super::charset::CHARSET;

/// Default model path — relative to the i12377_api project root.
const DEFAULT_MODEL: &str = "models/common_old.onnx";

/// Target image height (the model expects 64).
const TARGET_HEIGHT: u32 = 64;

static MODEL: OnceLock<Option<Mutex<Session>>> = OnceLock::new();

/// Run OCR on a captcha PNG. Returns the recognized text or `None` on any
/// failure — orchestrator should retry.
pub fn recognize(image_bytes: &[u8]) -> Option<String> {
    let mutex = get_model()?;
    let mut session = mutex.lock().ok()?;
    let (data, w) = preprocess(image_bytes)?;
    let shape = [1usize, 1, TARGET_HEIGHT as usize, w as usize];

    let input = TensorRef::from_array_view((shape, &data[..])).ok()?;
    let outputs = session.run(ort::inputs![input]).ok()?;
    // session mutex guard drops here at end of block
    let tensor: Tensor<f32> = outputs[0]
        .clone()
        .downcast::<TensorValueType<f32>>()
        .ok()?;
    let view = tensor.extract_array();

    // Output shape is dynamic; in ddddocr's CRNN it is (T, batch, C).
    // Collapse the batch dim (always 1) to get (T, C).
    let shape = view.shape().to_vec();
    tracing::debug!(?shape, "ocr output shape");
    let t = shape.first().copied().unwrap_or(1);
    let c = *shape.last().unwrap_or(&0);
    let owned: Array2<f32> = view
        .to_owned()
        .into_shape_with_order((t, c))
        .ok()
        .or_else(|| view.to_owned().into_shape((t, c)).ok())?;
    let text = ctc_decode(owned.view());
    tracing::debug!(text = %text, "ocr decoded");
    if text.is_empty() {
        return None;
    }
    Some(text)
}

/// Preprocess PNG → flat f32 buffer (1, 1, H, W) normalized to [0, 1].
fn preprocess(image_bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    let img = image::load_from_memory(image_bytes).ok()?.to_luma8();
    let (w0, h0) = img.dimensions();
    let ratio = TARGET_HEIGHT as f32 / h0.max(1) as f32;
    let w = ((w0 as f32 * ratio).round() as u32).clamp(8, 1024);
    let resized = image::imageops::resize(&img, w, TARGET_HEIGHT, FilterType::Lanczos3);

    let mut data = Vec::with_capacity((TARGET_HEIGHT * w) as usize);
    for y in 0..TARGET_HEIGHT {
        for x in 0..w {
            let p = resized.get_pixel(x, y).0[0];
            data.push(p as f32 / 255.0);
        }
    }
    Some((data, w))
}

/// CTC greedy decode.
fn ctc_decode(view: ndarray::ArrayView2<f32>) -> String {
    let mut out = String::with_capacity(view.len_of(ndarray::Axis(0)));
    let mut prev: usize = usize::MAX;
    for row in view.rows() {
        let (idx, _) = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or((0, &f32::NEG_INFINITY));
        if idx == 0 || idx == prev {
            continue;
        }
        if idx < CHARSET.len() {
            out.push(CHARSET[idx]);
        }
        prev = idx;
    }
    out
}

fn model_path() -> PathBuf {
    std::env::var("DDDOCR_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_MODEL))
}

fn get_model() -> Option<&'static Mutex<Session>> {
    MODEL.get_or_init(load_model).as_ref()
}

fn load_model() -> Option<Mutex<Session>> {
    let path = model_path();
    if !Path::new(&path).exists() {
        warn!(?path, "OCR model not found");
        return None;
    }
    match Session::builder() {
        Ok(mut builder) => match builder.commit_from_file(&path) {
            Ok(s) => Some(Mutex::new(s)),
            Err(e) => {
                warn!(error = %e, "failed to load OCR model");
                None
            }
        },
        Err(e) => {
            warn!(error = %e, "failed to create session builder");
            None
        }
    }
}