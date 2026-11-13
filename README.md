# SU-03T Tools

简体中文 | [English](README.en.md)

面向 **SU-03T / 蜂鸟 M** 的串口固件升级与调试工具，使用 Rust 构建原生桌面界面和 CLI，支持 macOS、Linux。

[![CI](https://github.com/MatriQ/su03t-macos/actions/workflows/check.yml/badge.svg)](https://github.com/MatriQ/su03t-macos/actions/workflows/check.yml) [![MIT](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

![中文升级界面](docs/images/transfer-zh.png)

*截图来自原生应用的模拟预览，不连接物理串口，不代表实机烧录验收。*

## 功能

- **固件升级**：文件与 SHA-256 检查、启动握手、按设备 ACK 推进的进度、超时重试和结束确认。
- **上下电引导**：明确区分“先断电”“串口就绪后上电”“保持供电”“完成后重新上电验证”。
- **常驻日志**：右侧通高面板、可拖动宽度、紧凑行距；上翻暂停跟随，到底部自动恢复；支持清除、复制和保存。
- **串口调试**：被动监听、HEX / UTF-8 显示、显式手动发送、接收 BIN 导出和滚动记录。
- **中英界面**：中文 / English / 系统语言，记住选择；切换不重置连接、进度或日志阅读位置。

## 安装

从 [Releases](https://github.com/MatriQ/su03t-macos/releases) 下载 [macOS 通用包 v0.5.0](https://github.com/MatriQ/su03t-macos/releases/download/v0.5.0/SU03T-Tools-v0.5.0-macOS-universal.zip)，解压后打开 `SU03T Update.app`。包含 Apple Silicon 和 Intel 两个架构；应用使用临时签名，未公证，首次打开请按 macOS 的标准流程确认信任来源。校验文件随发行包提供。

Linux 请按下方步骤从源码构建。macOS 构建目标为 12.0，具体系统、板型和驱动兼容性以实际测试为准，见[验证范围](docs/validation.md)。

## 接线与模式

**串口信号是 3.3V 电平，模块通常独立使用 5V 供电。两者不要混淆。** 独立供电时转接器 VCC 不接，必须共地；只切换模块电源，USB 转串口保持连接电脑。具体电源能力和引脚映射以开发板与固件为准。

| 模式 | 默认参数 | 模块与 USB 转串口接线 |
| --- | --- | --- |
| 固件升级 / 启动诊断 | 921600 / 8N1 / 无流控 | B7 → RXD、B6 ← TXD、GND ↔ GND |
| 小车语音调试预设 | 9600 / 8N1 / 无流控 | B7 → RXD、B6 ← TXD、GND ↔ GND |
| B8 启动日志 | 57600 / 8N1 / 只接收 | B8 → RXD、GND ↔ GND，TXD 不接 |

小车预设对应特定应用固件，不是所有 SU-03T 的统一业务参数。可选择通用模式并修改串口设置。工具不自动安装驱动，也不通过 DTR/RTS 控制模块复位。

## 固件升级

1. 先将模块断电，选择实际串口和平台导出的 `*_update.bin`。悬停文件名可查看路径，高级设置中可查看 SHA-256。
2. 点击 **模块已断电，开始连接**。工具先检查文件，再打开串口。
3. 出现 **现在给模块上电** 后，接通模块电源。USB 转串口保持连接。
4. 收到启动握手后自动传输。保持供电，观察右侧日志中的等待、重试和确认。
5. 全部数据达到 100% 后仍需等待设备结束确认。收到完成提示后，将模块断电再上电，验证唤醒词及语音指令。

![明确的上电提示](docs/images/power-zh.png)

升级需要兼容的蜂鸟 M 启动程序。默认重复发送 `32`（ASCII 字符 `2`）作为启动触发；高级设置可关闭。未收到 `CCC` 前不传固件。`release.bin` 完整镜像不能直接替代串口升级文件。

### 停在首包确认时

界面会显示等待时长、当前重试次数及本轮剩余时间。默认每次等待 5 秒，最多重试 30 次；连续超时后给出明显提示，可继续等待，或先停止，再检查 B6/B7、共地、供电和串口连接后重试。

![首包超时与重试提示](docs/images/retry-zh.png)

**清除日志只清空显示**，不停止烧录，也不重置进度、计数或故障上下文。上翻查看历史时，新日志不会抢走阅读位置；滚动到底部或点击“回到底部并跟随”可恢复跟随。长行可横向滚动。

“启动诊断 · 不传固件”无需选择文件，默认只接收；可显式启用启动触发。它不会发送固件、EOT 或猜测的擦除、复位命令。

## 串口调试与记录

连接后默认只监听。发送必须手动触发，支持 HEX 和 UTF-8；不自动添加帧头、帧尾或文本结尾。B8 模式在通信核心禁止发送，适合观察上电日志。

- 调试显示缓冲最多 2000 条 / 1 MiB，独立 RX 缓冲最多 1 MiB。清除显示保留会话计数、RX 缓冲及记录文件。
- 复制 / 保存导出当前显示记录；接收 BIN 导出保留的原始字节。设备输出、HEX、路径、文件名和校验值不随语言切换改写。
- 连续记录创建 `.rx.bin` 和 `.log`。每流最多四个 16 MiB 文件，超限滚动，新连接创建新文件，不覆盖其他会话。
- TX 计数表示主机写入，不表示模块已确认；串口读取分块不等于协议帧。

## CLI

无参数打开 GUI，以下命令使用同一个通信核心：

```bash
su03t-update ports
su03t-update firmware-info --firmware firmware_update.bin
su03t-update monitor --port /dev/ttyUSB0 --profile custom --baud 115200
su03t-update monitor --port /dev/ttyUSB0 --profile b8 --duration 60 --record ./session-001
su03t-update probe --port /dev/ttyUSB0 --baud 921600 --wait 15
su03t-update flash --port /dev/ttyUSB0 --firmware firmware_update.bin
```

macOS 使用实际枚举的 `/dev/cu.*`。`probe` 默认被动监听，需要启动触发时显式加 `--trigger`。`monitor` 仅在显式指定 `--send-hex` 或 `--send-text` 时发送；小车预设为 `--profile car`。更多参数见 `--help`。

## 从源码构建

需要 Rust stable 和本机 C 编译工具链；本项目使用 Rust 1.95 验证。Linux 还需 libudev 以及桌面相关开发库。Debian / Ubuntu 示例：

```bash
sudo apt-get install build-essential pkg-config libudev-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libwayland-dev libgl1-mesa-dev libegl1-mesa-dev \
  libx11-dev libxcursor-dev libxi-dev libxrandr-dev libxinerama-dev fonts-noto-cjk
```

```bash
cargo build --locked --release
cargo build --locked --release --no-default-features  # 仅 CLI
bash scripts/package-macos.sh                      # 本机 .app
rustup target add aarch64-apple-darwin x86_64-apple-darwin
bash scripts/package-macos.sh --universal           # macOS 通用 .app
bash scripts/package-linux.sh                     # 在 Linux 上打包
```

打包脚本需要 Python 3 和 Rust 文档组件（`rustup component add rust-docs`），以附带依赖与标准库的许可证。首次打包会从依赖记录的上游提交获取缺失的许可证文件，并缓存在 `target/`。

中文显示需要可用的中文字体。Linux 串口权限由系统配置决定；检查 dialout/uucp 组或设备权限，工具不会使用 sudo 打开串口。

## 测试与兼容范围

```bash
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo test --locked --no-default-features
cargo build --locked
python3 tests/client_acceptance.py target/debug/su03t-update
python3 tests/transfer_events.py target/debug/su03t-update
# 以下两项使用 macOS PTY 测试适配器
python3 tests/simulate_device.py target/debug/su03t-update
python3 tests/handshake_diagnostics.py target/debug/su03t-update
```

测试使用虚拟串口，不打开已连接的 USB 设备。覆盖包格式、CRC、序号回绕、重传、取消、文件变化拒绝、缺失结束确认、日志清除与自动跟随等。参见[协议说明](docs/protocol.md)、[验证记录](docs/validation.md)及[贡献指南](CONTRIBUTING.md)。

串口升级仍是实验性功能，不能默认用于其他型号。`CCC` 不识别型号，EOT ACK 只确认传输结束，**不等于读回校验、正常启动或语音功能验证**。请始终使用已确认适配的固件和启动程序。

## 许可证与资料

原创代码使用 [MIT License](LICENSE)。仓库不包含厂商固件、Windows 工具或驱动；第三方依赖和可选厂商资料保留各自许可证。本项目不是厂商官方客户端。

[SU-03T 模组资料](https://docs.aimachip.com/zh-cn/latest/file/offlineVoice/SU03T.html) · [蜂鸟 M FAQ](https://doc.hivoice.cn/udp/%E5%B8%B8%E8%A7%81%E9%97%AE%E9%A2%98.html)
