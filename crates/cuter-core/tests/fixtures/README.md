# 测试用的真实数据

这些文件不是手写的，是从**真东西**上抓下来的，用来把契约钉在现实上而不是我们的想象上。

## `paddle_real_response.json`

- **来源**：本机 PaddleOCR 2.9.1（paddlepaddle 2.6.2）跑 `tools/probe/paddle_ocr_server.py`，
  对 `docs/design/shots/0.1.49-ocr-row-hover.png` 的真实输出。
- **抓取方式**：向 `http://127.0.0.1:8917/ocr` POST 该图（multipart + base64），把响应原样存下来
  （脚本见提交 `0b0f5c1` 之前的 `target/tmp/capture_paddle_fixture.py`；`target/` 不进仓库，
  所以这里记清楚参数：同一张图、同一个服务、`result[0]` 就是 14 个块的那个版本）。
- **内容特征**：14 个块、词级四点框、置信度 0.9559–0.9996、含中文与全角标点
  （`单击图中的框可复制该行文字；切到「逐行对照」…`）。
- **为什么值得留**：手写 fixture 只能证明"我们以为 hubserving 长这样"；这个文件能证明
  "真引擎给的就是这样"。`parse_paddle_response` 的嵌套形状、坐标顺序、中文编码都靠它守。
- **用它的测试**：`crates/cuter-core/src/ocr/http_ocr.rs`
  的 `it_parses_the_response_a_real_paddle_server_sent` 与 `a_low_min_score_filters_the_real_payload`。
