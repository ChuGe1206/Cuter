# i18n 工具链

界面多语言的文案改造工具。所有脚本都是"生成 + 校验"配对：改完必须跑单测与探针。

| 脚本 | 作用 |
|---|---|
| `wrap_ui_strings.py` | 把**用户看得见**的调用点里的中文字面量包成 `tr!(…)`。跳过日志、注释、测试模块；支持跨行调用、匹配臂、自定义 UI 助手 |
| `i18n_convert.py` | 单行 `format!(tr!("…"), 参数)` → `fill!(tr!("…"), 参数)`（按括号配平解析） |
| `fix_multiline_format.py` | 跨行的同类改写（`format!(` 换行之后才是 `tr!("…")`） |
| `fix_fill_args.py` | 修 `fill!` 的参数形态（去掉 `[&…]` 包一层、多余的 `&`） |
| `extract_keys.py` | 扫源码里所有 `tr!("…")` 的键 → `target/i18n-keys.json` |
| `en_table.py` | 英文对照表（人写）+ 覆盖率检查（缺翻译直接非零退出） |
| `splice_table.py` | 把对照表**原地幂等**替换进 `crates/cuter-ui/src/i18n.rs` |

用法（本仓库自带 Python 是 `dsh` 的运行时，PATH 里没有 `python`）：

```powershell
$py = 'python'   # any Python 3.11+ on PATH works; or set $env:CUTER_PYTHON
$env:PYTHONIOENCODING = 'utf-8'
& $py tools\i18n\wrap_ui_strings.py            # dry-run 先看 diff
& $py tools\i18n\wrap_ui_strings.py --write
& $py tools\i18n\fix_multiline_format.py --write
& $py tools\i18n\extract_keys.py
& $py tools\i18n\en_table.py                   # 缺哪条会列出来
& $py tools\i18n\splice_table.py --write
cargo test -p cuter-ui                          # 四个兜底测试必须全绿
```

## 踩过的坑（都写进了脚本注释）

* **别用正则切参数**：`{:.0}`、具名捕获 `{count}`、嵌套括号会把正则切坏，
  第一版连 `RichText::new(...)` 的右括号都吃掉了。一律用括号配平解析。
* **排序按运行时字符串**：源码里的 `\n` 是两个字符，运行时是换行；按字面量排会让
  二分查找找不到带转义的键。
* **拼接表体必须幂等**：手工拼接把 `];` 拼重过一次，`splice_table.py` 现在整段替换。
* **`.ps1` 探针保持纯 ASCII**：PowerShell 5.1 读没有 BOM 的脚本会按 GBK 解析，
  中文会直接把语法搞坏。所以缺翻译的日志带了 `i18n-missing` 这个 ASCII 标记。
