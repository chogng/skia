# Windows Vulkan validation runbook

这份文档用于在 Windows 真机上验证 `skia-vulkan`。命令均在仓库根目录的
Developer PowerShell 中执行，可以按顺序直接复制。

当前测试范围是 Vulkan loader、instance、physical/logical device、graphics queue、
离屏 RGBA8 image、`Clear`、同步提交和 staging readback。它不创建窗口或 swapchain；
`FillRect`、`FillPath`、`StrokePath`、`DrawImage`、glyph 和复杂裁剪尚未纳入这一阶段。

## 1. 准备环境

必须安装：

1. 支持 Vulkan 的 Intel、AMD 或 NVIDIA 显卡驱动。优先使用显卡厂商当前驱动，不要只依赖
   Windows Update 提供的基础显示驱动。
2. Rust stable toolchain。仓库根目录的 `rust-toolchain.toml` 会选择具体工具链。
3. Visual Studio Build Tools 的 **Desktop development with C++** workload。推荐从
   “Developer PowerShell for VS”运行下方命令。

基础测试通过 `ash` 动态加载系统的 `vulkan-1.dll`，不要求 Vulkan SDK。第二轮
Validation Layer 测试需要安装 Vulkan SDK，并包含 `VK_LAYER_KHRONOS_validation`。

先进入仓库根目录并确认工具链：

```powershell
Set-Location C:\path\to\skia
rustc -Vv
cargo -V
Get-Item "$env:WINDIR\System32\vulkan-1.dll"
```

如果最后一条命令找不到文件，先安装或更新显卡驱动，不要继续运行测试。

## 2. 运行基础真机测试

先清除可能遗留的环境变量，然后强制测试必须取得真实 Vulkan device：

```powershell
Remove-Item Env:SKIA_VULKAN_VALIDATION -ErrorAction SilentlyContinue
Remove-Item Env:RUST_BACKTRACE -ErrorAction SilentlyContinue
$env:SKIA_REQUIRE_VULKAN_DEVICE = "1"
cargo test -p skia-vulkan -- --nocapture --test-threads=1
```

成功时必须同时满足：

- 输出包含 `running 2 tests`；
- 输出包含 `Vulkan device: <设备名称>`；
- `vulkan_backend_clears_and_reads_an_offscreen_surface` 为 `ok`；
- `vulkan_backend_fails_closed_for_unimplemented_draws` 为 `ok`；
- 最终为 `2 passed; 0 failed`。

第二个测试中的 `UnsupportedCommand` 是当前契约的一部分：测试会确认未实现的 draw 明确
失败，而不是偷偷回退到 CPU。只要测试本身显示 `ok`，它就不是异常。

## 3. 运行 Validation Layer 测试

安装 Vulkan SDK 后，建议先确认 SDK 和 layer 可见：

```powershell
vulkaninfo --summary
```

然后在同一个 Developer PowerShell 中运行：

```powershell
$env:SKIA_REQUIRE_VULKAN_DEVICE = "1"
$env:SKIA_VULKAN_VALIDATION = "1"
cargo test -p skia-vulkan -- --nocapture --test-threads=1
```

成功标准仍是 `2 passed; 0 failed`。此外，测试会确认 backend 确实启用了
`VK_LAYER_KHRONOS_validation`。如果 layer 不存在，测试必须以
`ValidationUnavailable` 失败，不能静默跳过。

当前 backend 只启用 validation layer，还没有注册 `VK_EXT_debug_utils` callback。因此此轮
主要验证 layer 可创建且 Vulkan 操作成功；完整 validation message 捕获会在后端加入 debug
messenger 后补充。

## 4. 失败时收集诊断

任何一轮失败后，用 backtrace 单线程重跑失败轮次：

```powershell
$env:RUST_BACKTRACE = "1"
cargo test -p skia-vulkan -- --nocapture --test-threads=1
```

如果是 Validation Layer 轮次，保留 `SKIA_VULKAN_VALIDATION=1`；如果是基础轮次，先删除它：

```powershell
Remove-Item Env:SKIA_VULKAN_VALIDATION -ErrorAction SilentlyContinue
```

错误代码与建议处理方式：

| 错误 | 含义 | 先检查什么 |
| --- | --- | --- |
| `LoaderUnavailable` | 无法加载 `vulkan-1.dll` | 更新厂商显卡驱动，重新检查 System32 中的 loader |
| `InstanceCreationFailed` | loader 存在但 instance 创建失败 | 更新驱动；若仅 validation 轮失败，检查 SDK/layer 与驱动兼容性 |
| `ValidationUnavailable` | 找不到 Khronos validation layer | 安装或修复 Vulkan SDK，确认 `vulkaninfo --summary` 可运行 |
| `DeviceUnavailable` | 没有 graphics-capable physical device/queue | 检查虚拟机、远程桌面会话和显卡驱动是否暴露 Vulkan |
| `DeviceCreationFailed` | logical device、queue 或 command pool 创建失败 | 更新驱动并保留完整 backtrace |
| `SurfaceAllocationFailed` | RGBA8 image、格式能力或 device memory 不可用 | 记录 GPU/驱动版本并保留完整输出 |
| `SubmissionFailed` | command recording、queue submit 或 fence wait 失败 | 保留完整输出和 backtrace |
| `ReadbackFailed` | staging buffer、copy、mapping 或像素读回失败 | 保留完整输出和 backtrace |

如果 `cargo` 在运行测试前就提示 `link.exe`、Windows SDK 或 C/C++ 工具缺失，这不是 Vulkan
错误；请回到 Visual Studio Installer 补装 **Desktop development with C++** workload。

## 5. 回传结果

请把下面信息以文本形式发回，不需要截图：

```powershell
rustc -Vv
cargo -V
Get-CimInstance Win32_VideoController | Select-Object Name, DriverVersion
vulkaninfo --summary
```

其中 `vulkaninfo` 只在已经安装 Vulkan SDK 时运行。还需要附上：

1. 基础真机测试的完整输出；
2. Validation Layer 测试的完整输出；
3. 若失败，带 `RUST_BACKTRACE=1` 的重跑输出；
4. 测试是在本机桌面、Remote Desktop、虚拟机还是 CI runner 中执行。

不要只发送“通过”或最后一行，因为设备名和具体失败阶段会决定后续 Vulkan 实现是否需要
兼容处理。

## 6. 清理环境变量

测试结束后执行：

```powershell
Remove-Item Env:SKIA_REQUIRE_VULKAN_DEVICE -ErrorAction SilentlyContinue
Remove-Item Env:SKIA_VULKAN_VALIDATION -ErrorAction SilentlyContinue
Remove-Item Env:RUST_BACKTRACE -ErrorAction SilentlyContinue
```

基础轮和 Validation Layer 轮都通过，表示当前 Vulkan 离屏 clear/readback foundation 在该
Windows GPU/驱动组合上成立；它不代表尚未实现的绘制命令或窗口呈现已经通过。
