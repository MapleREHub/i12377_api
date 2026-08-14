//! Captcha solver — pure Rust ONNX inference + math evaluation.
//!
//! Pipeline:
//!   1. [`onnx::recognize`] runs ddddocr's CRNN model (via `tract-onnx`,
//!      pure Rust) on the raw PNG bytes and returns the recognized text.
//!   2. [`evaluate`] filters to the math charset (`0-9 + - * / = ?`) and
//!      computes the integer answer.

pub mod charset;
pub mod onnx;

/// Allowed math expression characters after OCR.
const ALLOWED: &[char] = &['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '+', '-', '*', '/', '=', '?', '？', '×', '÷', 'x', 'X'];

/// Recognize a math captcha PNG and return the integer answer.
///
/// Returns `None` on any failure (model missing, OCR confidence too low,
/// expression not parseable). The orchestrator retries on `None`.
pub fn recognize(image_bytes: &[u8]) -> Option<String> {
    let raw = onnx::recognize(image_bytes)?;
    evaluate(&raw)
}

/// Extract digits and operator from OCR output, evaluate the expression.
///
/// The model output may include non-math junk characters; we keep only
/// `0-9 + - * / = ?` (plus their fullwidth / ascii variants), expect the
/// pattern `digit op digit = ?`, and compute the integer answer.
fn evaluate(raw: &str) -> Option<String> {
    let kept: String = raw
        .chars()
        .filter(|c| ALLOWED.contains(c))
        .collect();

    // Normalize all variants to ASCII.
    let s: String = kept
        .chars()
        .map(|c| match c {
            'x' | 'X' | '×' => '*',
            '÷' => '/',
            '？' => '?',
            '=' => '=',
            c => c,
        })
        .collect();

    // Find first "<digit><op><digit>" triple; '=' and '?' are optional.
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        let a = bytes[i];
        let op = bytes[i + 1] as char;
        let b = bytes[i + 2];
        if a.is_ascii_digit() && matches!(op, '+' | '-' | '*' | '/') && b.is_ascii_digit() {
            let an = (a - b'0') as i64;
            let bn = (b - b'0') as i64;
            let result = match op {
                '+' => an + bn,
                '-' => an - bn,
                '*' => an * bn,
                '/' if bn != 0 => an / bn,
                '/' => 0,
                _ => return None,
            };
            return Some(result.to_string());
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_add() {
        assert_eq!(evaluate("5+6=?").as_deref(), Some("11"));
        assert_eq!(evaluate("5+6").as_deref(), Some("11"));
    }

    #[test]
    fn eval_sub() {
        assert_eq!(evaluate("9-3=?").as_deref(), Some("6"));
    }

    #[test]
    fn eval_mul() {
        assert_eq!(evaluate("4*7=?").as_deref(), Some("28"));
    }

    #[test]
    fn eval_div() {
        assert_eq!(evaluate("8/2=?").as_deref(), Some("4"));
    }

    #[test]
    fn eval_with_junk() {
        assert_eq!(evaluate("AB5+6=?XY").as_deref(), Some("11"));
    }

    #[test]
    fn eval_x_to_mul() {
        assert_eq!(evaluate("4x7=?").as_deref(), Some("28"));
        assert_eq!(evaluate("4X7=?").as_deref(), Some("28"));
        assert_eq!(evaluate("4×7=?").as_deref(), Some("28"));
    }

    #[test]
    fn eval_fullwidth_q() {
        assert_eq!(evaluate("5+6=？").as_deref(), Some("11"));
    }

    #[test]
    fn eval_div_by_zero() {
        assert_eq!(evaluate("5/0=?").as_deref(), Some("0"));
    }

    #[test]
    fn eval_no_match() {
        assert_eq!(evaluate("??"), None);
        assert_eq!(evaluate("abc"), None);
    }
}