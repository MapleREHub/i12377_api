# 12377 举报查询 API · Rust 重构设计

**日期**: 2026-08-14
**作者**: Claude (brainstorming session)
**目标项目**: `E:\Projects\rehub\12377\i12377_api`
**参考项目**: `E:\Projects\rehub\12377_api`（Python FastAPI 实现）

---

## 1. 目标与范围

将 Python `12377_api` 服务以**极致性能 + 最小体积**为目标重写为 Rust 单二进制服务，保持 API 兼容（端点、请求/响应结构相同）。

**不在范围内**：
- 多 worker / 进程模式
- 验证码识别以外的 ML 推理
- 第三方分发（pip / crates.io 发布）
- 配置文件热加载

## 2. 架构

分层单进程：

```
HTTP API (axum 0.7)
  GET  /health
  POST /query
        │
        ▼
QueryOrchestrator (orchestrator.rs)
  - 重试循环（5× on captcha error 3104）
  - 状态：当前 attempt、session
        │
        ├──► HttpClient (client.rs, reqwest + rustls)
        │     - 创建 session、注入 guestKey cookie
        │     - GET captcha (PNG bytes + Set-Cookie JSESSIONID)
        │     - POST 查询 form-encoded
        │
        └──► Captcha Solver (captcha/*.rs, 纯 Rust)
              1. binarize
              2. 连通分量
              3. 几何特征分类数字
              4. 像素模式识别运算符
              5. 表达式求值
```

## 3. 模块布局

```
src/
├── main.rs            # tokio 启动 axum，订阅 tracing
├── config.rs          # env: HOST, PORT, MAX_RETRIES, RUST_LOG
├── routes.rs          # axum Router 注册端点
├── error.rs           # ApiError + IntoResponse
├── models.rs          # 请求/响应 DTO (serde)
├── client.rs          # reqwest session + cookie mgmt
├── orchestrator.rs    # do_query() 重试逻辑
└── captcha/
    ├── mod.rs         # recognize_captcha() 入口
    ├── binarize.rs    # PNG → Luma8 → bool mask
    ├── components.rs  # 8-邻域 flood fill → Vec<BBox>
    ├── digits.rs      # 几何特征分类 0-9
    ├── operators.rs   # 像素模式分类 + - × ÷
    └── eval.rs        # 表达式 → 答案

tests/
└── integration.rs     # mock 12377 响应，验证 do_query 行为
```

## 4. Cargo 依赖

```toml
[dependencies]
axum = "0.7"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "time"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "gzip"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
image = { version = "0.25", default-features = false, features = ["png"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["trace"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
chrono = { version = "0.4", default-features = false, features = ["clock"] }
rand = "0.8"
thiserror = "1"
once_cell = "1"

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
strip = true
panic = "abort"
```

**体积目标**: `cargo build --release` → ~3 MB；UPX 后 ~1 MB。

## 5. 数据模型

```rust
// models.rs
#[derive(Deserialize)]
pub struct QueryRequest {
    #[serde(rename = "retrieval_code")]
    pub retrieval_code: String,   // 1..=64 chars
}

#[derive(Serialize)]
pub struct ReportRecord {
    pub harm_type: String,
    pub retrieval_code: String,
    pub report_time: String,
    pub harm_url: String,
    pub result: String,
}

#[derive(Serialize)]
pub struct QueryResponse {
    pub success: bool,
    pub total: usize,
    pub records: Vec<ReportRecord>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,           // "ok"
    pub version: String,          // env!("CARGO_PKG_VERSION")
}
```

## 6. 错误模型

```rust
// error.rs
#[derive(thiserror::Error, Debug)]
pub enum ApiError {
    #[error("retrieval code cannot be empty")]
    EmptyCode,
    #[error("retrieval code too long (max 64)")]
    CodeTooLong,
    #[error("upstream network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("upstream returned non-JSON")]
    BadJson,
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            ApiError::EmptyCode | ApiError::CodeTooLong => (400, self.to_string()),
            ApiError::Network(_) => (502, "upstream unreachable".into()),
            ApiError::BadJson => (502, "bad upstream response".into()),
        };
        (StatusCode::from_u16(status).unwrap(), Json(json!({"error": msg}))).into_response()
    }
}
```

## 7. HTTP 客户端细节

- `reqwest::Client`（非 Session） + 手动维护 cookie jar (`Arc<Mutex<HashMap<String, Cookie>>>`)
- 启动时：
  1. `GET https://www.12377.cn/jbcx.html?tab=6` → 拿初始 cookie + Referer
  2. 注入 `guestKey = YYYYMMDDHHmmss + 6位 A-Z0-9`（domain `.12377.cn`）
- 获取验证码：
  - `GET https://new.12377.cn/rpapi/portal/captcha?{unix_ms}`
  - 响应 `Set-Cookie: JSESSIONID=...` → 解析存入 cookie jar
- 提交查询：
  - `POST https://new.12377.cn/rpapi/portal/report/get`
  - form: `retrievalCode={code}&verifyCode={answer}&pageSize=1000`
  - `Content-Type: application/x-www-form-urlencoded`
- User-Agent: Chrome 131 (Windows)

## 8. 验证码求解（纯几何）

### 8.1 二值化
- 解码 PNG → `ImageBuffer<Luma8, Vec<u8>>`
- 阈值：像素 < 200 视为前景（黑色），否则白
- 输出：紧凑 `Vec<u8>` 掩码（1 bit/像素，64 字节对齐行）

### 8.2 连通分量
- 8-邻域 flood fill（迭代栈，零递归 → 防栈溢出）
- 输出 `Vec<BBox>` 按 x 排序，过滤面积 < 100 px²
- 若 < 3 个分量 → 视为识别失败

### 8.3 数字几何分类（digits.rs）

对每个连通分量计算：
- `loop_count`: 内部白色空洞数（0/8→2; 0/4/6/9→1; 其余→0）
- `endpoint_count`: 度数为 1 的边界像素数
- `aspect_ratio`: height/width
- `density_profile`: 行/列前景密度峰值

查表：

| 数字 | loops | endpoints | 其他特征 |
|------|-------|-----------|----------|
| 0    | 1     | 0         | 瘦高，aspect > 1.4 |
| 1    | 0     | 2         | 最瘦，aspect > 2.0 |
| 2    | 0     | 2         | 顶部水平密度峰值，底部右倾 |
| 3    | 0     | 2         | 右侧双凹（左列无峰值） |
| 4    | 0     | 3         | 上半水平密度峰值 + 右竖线 |
| 5    | 0     | 2         | 上半水平密度峰值 + 下半左凸 |
| 6    | 1     | 1         | loop 在底部 |
| 7    | 0     | 2         | 顶部水平密度峰值，无底部特征 |
| 8    | 2     | 0         | 双峰行密度 |
| 9    | 1     | 1         | loop 在顶部 |

置信度：所有特征严格匹配才返回数字；否则返回 `None`。

### 8.4 运算符识别（operators.rs）
直接移植 Python `_detect_operator` 像素模式：
- 中心列 + 中心行 密度 → `+`
- 仅中心行 → `-`
- 双向对角线密度 → `*`
- 单向对角线 → `/`

### 8.5 求值
- 解析 `digit op digit = ?` → `i32`
- `+ - * //` 左到右
- 返回 `Some(answer.to_string())` 或 `None`

## 9. 重试编排

```rust
pub async fn do_query(code: &str) -> Result<Vec<ReportRecord>, OrchError> {
    let client = HttpClient::new().await?;
    for attempt in 1..=MAX_RETRIES {
        let img = client.fetch_captcha().await?;
        let answer = match captcha::recognize(&img) {
            Some(a) => a,
            None => { tracing::warn!(attempt, "captcha recognize failed"); continue; }
        };
        match client.submit_query(code, &answer).await {
            Ok(resp) if resp.code == SUCCESS_CODE => return Ok(parse(resp)),
            Ok(resp) if resp.code == CAPTCHA_ERR => continue,
            Ok(resp) => return Err(OrchError::BadCode(resp.code)),
            Err(e) if attempt < MAX_RETRIES => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(OrchError::Exhausted)
}
```

## 10. 测试策略

- **单元测试**：每个 captcha 子模块测试向量（手写二值化 mask → 期望数字）
- **集成测试**：`mockito` mock `new.12377.cn` 的 captcha + report 接口，验证 `do_query` 重试、cookie 流、成功/失败响应
- **二进制烟雾测试**：启动服务，`curl /health` 返回 200，`curl -X POST /query` 命中 mock 路径
- **基准测试**：`criterion` 测量 `recognize_captcha` 在 100 张合成图上的耗时

## 11. 配置

| Env 变量 | 默认 | 说明 |
|---------|------|------|
| `HOST` | `0.0.0.0` | 监听地址 |
| `PORT` | `8000` | 监听端口 |
| `MAX_RETRIES` | `5` | 验证码重试次数 |
| `RUST_LOG` | `info` | tracing filter |

## 12. 部署

```bash
# 开发
cargo run --release

# 生产构建
cargo build --release
upx --best --lzma target/release/i12377_api.exe   # 体积压到 ~1 MB

# 交叉编译（Linux）
cargo build --release --target x86_64-unknown-linux-musl
```

## 13. 风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| 几何分类在某些字体下识别率低 | 查询成功率下降 | 失败 → 整体重试，与 Python 一致 |
| reqwest rustls 在某些老 Windows 缺根证书 | 首次 TLS 失败 | 文档说明需 ca-certificates；不打包根证书以省体积 |
| UPX 压缩后启动慢 ~50ms | 冷启动 | 用户可关；UPX 是可选步骤 |
| 验证码字体变化 | 求解全部失败 | 文档化为已知限制，需更新特征表 |

## 14. 验收标准

- [ ] `cargo build --release` 成功，二进制 ≤ 4 MB（未压缩）/ ≤ 1.5 MB（UPX 后）
- [ ] `GET /health` 返回 `{"status":"ok","version":"0.1.0"}`
- [ ] `POST /query` 在 mock 上正确：
  - 验证码错误 3104 → 重试
  - 验证码正确 1000 → 返回 records
  - 5 次失败 → 返回 `success:false, error:"..."`
- [ ] 单元测试覆盖所有数字 0-9 的几何特征向量
- [ ] 集成测试 mock 整个 12377 流程
- [ ] 真实环境手动测试：至少 1 条 `retrieval_code` 查询成功