# i12377_api

Rust port of [`12377_api`](../12377_api) (Python FastAPI) — 12377.cn 举报查询服务。

Single-binary, zero-OCR, **~1 MB** after UPX compression.

## Why Rust

- **极致性能** — axum + Tokio + reqwest(rustls) 异步栈；几何验证码求解同步但仅 ~ms 级。
- **零外部依赖** — 单文件 exe（Windows）或 ELF（Linux musl），无 Python 运行时、无 ddddocr 模型文件。
- **极小体积** — `opt-level="z"` + LTO + `strip` + `panic="abort"` 后 2.4 MB，UPX 压缩到 **~1 MB**。

## Captcha solving — pure geometry

No OCR, no ML. The captcha format is always `digit [+|-|*|/] digit = ?`. The solver pipeline:

1. **Binarize** — decode PNG → packed 1-bit mask (foreground = ink).
2. **Connected components** — 8-neighborhood flood-fill, returns ≥ 3 boxes sorted left-to-right.
3. **Geometric digit classifier** — count closed loops + endpoints in each digit bbox → match table:

   | Digit | Loops | Endpoints | Heuristic |
   |-------|-------|-----------|-----------|
   | 0 | 1 | 0 | tall narrow bbox |
   | 1 | 0 | 2 | tallest, aspect > 2 |
   | 2 | 0 | 2 | top-heavy row |
   | 3 | 0 | 2 | mid aspect, no top/bottom heavy |
   | 4 | 0 | 3 | horizontal top + vertical right |
   | 5 | 0 | 2 | top + bottom heavy rows |
   | 6 | 1 | 1 | loop bottom (lower half denser) |
   | 7 | 0 | 2 | top heavy only |
   | 8 | 2 | 0 | double loop |
   | 9 | 1 | 1 | loop top (upper half denser) |
4. **Operator detector** — center column/row density + diagonal scan (ported from Python reference).
5. **Evaluate** — `a op b` → integer answer.

If recognition fails the orchestrator retries with a fresh captcha (5× max).

## Build

```bash
# Debug
cargo run

# Release (size-optimized)
cargo build --release

# Optional: compress further
upx --best --lzma target/release/i12377_api.exe
```

| Stage | Size (Windows x64) |
|-------|--------------------|
| `cargo build --release` | ~2.4 MB |
| + `upx --best --lzma`   | **~1.0 MB** |

## Run

```bash
PORT=8000 RUST_LOG=info ./target/release/i12377_api.exe
```

Env vars:
- `HOST` (default `0.0.0.0`)
- `PORT` (default `8000`)
- `MAX_RETRIES` (default `5`)
- `CAPTCHA_TIMEOUT` (default `15s`)
- `QUERY_TIMEOUT` (default `30s`)
- `RUST_LOG` (default `info`)

## API

### `GET /health`

```json
{"status":"ok","version":"0.1.0"}
```

### `POST /query`

```bash
curl -X POST http://localhost:8000/query \
  -H 'Content-Type: application/json' \
  -d '{"retrieval_code":"H026061219520245669A"}'
```

Success:
```json
{
  "success": true,
  "total": 1,
  "records": [
    {
      "harm_type": "...",
      "retrieval_code": "...",
      "report_time": "2024-06-12",
      "harm_url": "...",
      "result": "已处理"
    }
  ],
  "error": null
}
```

Failure (e.g. captcha retries exhausted):
```json
{
  "success": false,
  "total": 0,
  "records": [],
  "error": "captcha recognition failed after 5 attempts"
}
```

## Layout

```
src/
├── main.rs            # tokio + axum bootstrap
├── config.rs          # env loader
├── error.rs           # ApiError + IntoResponse
├── models.rs          # serde DTOs
├── routes.rs          # /health, /query handlers
├── orchestrator.rs    # do_query() retry loop
├── client.rs          # reqwest + manual cookie jar
└── captcha/
    ├── mod.rs         # recognize() entry
    ├── binarize.rs    # PNG → 1-bit mask
    ├── components.rs  # flood-fill labeling
    ├── digits.rs      # geometric classifier 0–9
    ├── operators.rs   # pixel-pattern operator detector
    └── eval.rs        # a op b → answer
```

## Known limits

- Captcha format must remain `digit op digit` (4 ops). Changing font or format requires re-tuning the digit table.
- TLS uses bundled rustls roots — system CA store is ignored.
- No session pooling — every request creates a fresh `guestKey`. (Pooling would need careful JSESSIONID binding handling.)