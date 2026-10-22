# 配置项 ↔ 代码作用点审计

> **为什么会有这份文档**：本项目的硬约束之一是「设置里的开关必须真的生效」
> （见 [README](../README.md) 的「设计原则」一节）。而"某个字段到底有没有被读"
> 这件事，靠读代码是很容易看漏的——`crates/cuter-core/src/config.rs` 里每个字段
> 都长得一样，谁被用、谁只是占位，只有把整个 workspace 搜一遍才知道。
>
> 这份表最早是 **2026-09-28 那一轮全量审计的产物**，2026-10-17（0.1.51）重写了一遍：
> 上一版把同一行复制了四遍、又把三个已经落地的字段写成"已删除"，
> 与其继续打补丁不如按代码重新对一遍。

审计方法（可复现，一条命令）：

```powershell
# 每个配置字段在 config.rs 之外被引用几次（0 = 没人读，脚本以 1 退出）
python tools\docs\audit_config.py

# cuter-core 里 app/ui 一个都没调用的 pub fn（看"core 内部也没人调"那一列）
python tools\docs\audit_config.py --api
```

> 2026-10-17 之前的版本是手写的 `Select-String` + `grep` 两条命令。它们会**误判**：
> 字段名出现在注释或文档里也算"有人读"，`check_update` 当年就是这样被算成生效的。

---

## 1. 结论一览

| 分组 | 字段 | 状态与证据 |
|---|---|---|
| `general` | `theme` | 生效（`Auto` 真的读 `ctx.system_theme()`；`docs/design/DESIGN-SYSTEM.md` 有六张主题图） |
| `general` | `language` | 生效（启动时 `i18n::set_language`；`auto` 由 `desktop::system_language()` 探测；`i18n_e2e.ps1` + 四个单测兜住表完整性） |
| `general` | `start_in_tray` | 生效（0.1.9 修掉 eframe `post_rendering` 无条件显示根窗口；`single_instance_e2e.ps1` 实测窗口不可见。踩坑见 [HANDOFF 13.5](HANDOFF.md)） |
| `general` | `close_to_tray` | 生效（`handle_root_close`；`tray_close_e2e.ps1`） |
| `general` | `show_tray_notifications` | 生效（`TrayNotifier` + `Shell_NotifyIconW(NIM_MODIFY, NIF_INFO)`；`tray_notify_e2e.ps1` 双向：开→`delivered=True`，关→一条日志都没有） |
| `general` | `autostart` | 生效（`desktop::reconcile_autostart` → `HKCU\...\Run\Cuter`；`autostart_e2e.ps1` 四种情形，含"不许动别人写的项"） |
| `general` | `check_update` | 生效（0.1.47；`update_check_e2e.ps1` 四种情形 A–D：有新版本 / 关掉不发请求 / 地址留空不发请求 / 真实 https 不挂死） |
| `general` | `update_endpoint` | 生效（0.1.47；留空＝不检查，探针情形 C 证明一个字节都不发） |
| `hotkeys` | 9 项 | 生效（`desktop::Hotkeys::apply` 注册/注销；`toggle_pins` 的默认值从 `Shift+F3` 改成 `Alt+H` 是因为前者被本机常驻的截图类程序占着） |
| `capture` | `backend` | 生效（`desktop_capture_with`；启动时后台预热 DXGI，建不起来回退 GDI；`dxgi_backend_e2e.ps1` + 忽略测试 `dxgi_vs_gdi`） |
| `capture` | `snap_controls` | 生效（`overlay::control_snap_rect` → `uia::control_at`；`control_snap_e2e.ps1` 用存出来的 PNG 尺寸验证） |
| `capture` | `hide_own_windows` | 生效（`hide_all_windows`；`hide_own_windows_e2e.ps1` 像素级验证） |
| `capture` | `include_cursor` / `delay_ms` / `remember_last_region` / `last_region` / `show_magnifier` / `auto_detect_windows` / `play_sound` | 生效（`decorate_shot` / `advance_request` / `remember_region` / 遮罩还原 / `overlay.rs` / `MessageBeep`） |
| `annotate` | 9 项 | 生效（`AnnotateSettings::from_config`；`undo_limit` → `Doc::set_undo_limit`；`text_font` → `fonts::setup`） |
| `annotate` | 遮罩里改的样式 | 生效（`Overlay::take_style_change()` 旁路上报 → `apply_tool_style` 写回配置） |
| `save` | `dir` / `format` / `filename_pattern` / `auto_copy` / `auto_save` / `ask_path_on_save` / `reveal_after_save` / `quality` | 生效（`deliver` / `next_save_name` / `image_format()` → `with_quality`） |
| `pin` | `opacity` / `click_through` / `border` / `always_on_top` / `initial_scale` / `wheel_zoom` / `zoom_within_window` / `shadow` / `texture_budget_mb` | 生效（`Pin::new` / `zoom_by` / `desired_window_size` / `paint_shadow` / `warn_if_pin_textures_over_budget`） |
| `pin` | `rounded_corners` | 生效（`Pin::texture()` 上传前 `Frame::rounded_corners`；`pin_rounded_probe.ps1` 像素验证开/关） |
| `pin` | `groups` / `active_group` | 生效（`pin_should_show` 决定视口声明；`pin_groups_e2e.ps1` 六步全绿） |
| `ocr` | `engine` / `languages` / `auto_after_capture` / `auto_copy` / `paragraph` | 生效（`recognize_frame`；`languages` 的第一项交给 `WindowsOcr::new`） |
| `ocr` | `http_endpoint` / `min_score` | 生效（0.1.44 接通在线引擎：自写 HTTP + base64 + JSON，零新依赖；`ocr_http_e2e.ps1` + `tools/probe/fake_ocr_server.py`，含斜框 `?rotate=1` 与竖排 `?vertical=1`） |
| `scroll` | `auto_scroll` / `auto_stall_frames` | 生效（`scroll_worker` 按 `AutoScrollTracker` 的判定合成滚轮；`scroll_auto_e2e.ps1` 实测转发 35 次、长图 2413 像素后自己停） |
| `scroll` | `interval_ms` / `min_offset` / `max_offset` / `max_height` | 生效（采集线程间隔、`MatchOptions`；`max_height` 是**长度上限**——纵向＝高度、横向＝宽度，方向由 `DirectedStitcher` 自动判定，0.1.50 起横拼可用） |
| `history` | `limit` / `thumb_width` | 生效（`HistoryStore::add` 写盘并按 `limit` 淘汰；`history_e2e.ps1` 逐字节比对 + 淘汰 + 关闭后不写盘） |

审计脚本当前的输出是 **「所有字段都有读者。所有 pub fn 都有调用者。」**

---

## 2. 删掉的字段

按「没做的能力不给开关」的约定删除。它们既没有实现、也没有任何界面入口，
留着只会让人以为配置生效：

| 字段 | 曾经声称的能力 | 去哪了 |
|---|---|---|
| `ocr.model_dir` | 给"自带模型的引擎"预留的模型目录 | 2026-10-17（0.1.51）删除：系统 OCR 与在线接口都不需要模型文件，`resolved_model_dir()` 从未被调用 |

同一轮还删掉 8 个没有任何调用者的 `pub fn`（`geom.rs` 的 `to_f32` / `to_point` /
`local_to_global` / `global_to_local` / `bounding`、`config.rs` 的 `is_region_effect` /
`stroke_rgba`、`ocr/http_ocr.rs` 的 `with_timeout`）。它们都是"看起来有用、
其实一次都没被调用"的预留 API，`cargo clippy -D warnings` 抓不到（`pub` 项不会报 dead_code）。

**向后兼容**：删字段是安全的——`Config` 没有 `deny_unknown_fields`，
老配置里的键会被 serde 安静忽略。这一条有回归测试守着：
`config.rs::unknown_general_keys_are_ignored`（配置损坏永不导致启动失败是本项目的承诺）。

---

## 3. 现在没有"未实现但保留"的字段了

2026-09-28 那一版这里列着三个 `ocr` 字段（`min_score` / `model_dir` / `http_endpoint`），
理由都是"在线 HTTP OCR 引擎还没做"。0.1.44 接通在线引擎后前两个里的
`min_score`、`http_endpoint` 变成真的生效；`model_dir` 在 0.1.51 删除。
**如果以后又要加"预留字段"，请先读第 5 节第 4 条：先去路线图登记，不要加字段。**

---

## 4. 新加字段时的检查清单

1. 字段要有默认值，并加入 `Config::sanitize`（越界值夹取，手动编辑配置也不会崩）
2. 在 `app.rs::paint_control` 的某个 `section_*` 里给它一个控件，**并且**在代码里找到唯一的作用点
3. 跑一遍 `python tools\docs\audit_config.py`，确认引用次数不是 0
4. 如果这个作用点是"以后再做"，那就**先不要加字段**，去路线图里登记
5. 在这个文件里补一行，状态列写清楚证据（探针名 / 测试名）
6. **字段从"未实现"变成"已实现"时，回来改这一行**——过期的审计表比没有更危险，
   它给人一种"已经查过"的错觉（0.1.51 就是为此重写了整张表，见 HANDOFF 第 72 节）

---

## 5. ⚠️ 这张表的边界：它只能证明"有人读"，不能证明"行为对"

本表是**静态**审计的产物（搜字段名的消费者）。第二轮立刻撞上了它的盲区：
`start_in_tray` 明明有一个看起来完全合理的作用点（`main.rs` 里的
`ViewportBuilder::with_visible(!start_in_tray …)`），却**根本没有生效** ——
因为 eframe 画完第一帧之后会无条件把根窗口显示出来，把那个选项盖掉。

所以第 4 节之后还有一条，**只有它能给"生效"这两个字背书**：

> 用真实进程验证这条连线（点一下、发一次 `WM_CLOSE`、量一次窗口可见性），
> 把证据（命令 + 输出）写进提交信息或 HANDOFF。
> 凡是"只读代码就判定生效"的条目，状态列里写「代码路径成立，未实测」。

现在的实测手段（`tools/probe/run_all.ps1 -Quick` 会跑其中 20 个）：
`tray_close_e2e.ps1` / `single_instance_e2e.ps1` / `autostart_e2e.ps1` /
`hide_own_windows_e2e.ps1` / `tray_notify_e2e.ps1` /
`update_check_e2e.ps1` / `ocr_http_e2e.ps1` / `scroll_auto_e2e.ps1` /
`scroll_horizontal_e2e.ps1` / `control_snap_e2e.ps1` / `dxgi_backend_e2e.ps1` /
`pin_rounded_probe.ps1` / `pin_groups_e2e.ps1` / `history_e2e.ps1` 等。
