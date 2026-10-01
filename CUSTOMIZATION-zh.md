# 修改版说明

本分支基于 scrcpy-mask v0.9.0，保留上游 Apache-2.0 许可证。
新增手机指针映射、FPS 触摸及鼠标捕获改进和独立数据目录支持。

## 使用方式

1. 通过 USB 连接手机，开启 USB 调试并授权 ADB。
2. 启动手机指针辅助服务（见下方），启动 scrcpy-mask 并连接设备。可以关闭视频、音频。
3. 映射页面空白处右键，创建“手机指针”；设置切换按键，拖动标记设置每次呼出的起点，保存。
4. 按 FPS 映射的绑定键开启鼠标控制，再按手机指针绑定键切换指针／视角。
5. 指针模式左键发送触摸点击，按住移动支持拖动；再次按切换键恢复 FPS 视角。

F8 是常用 FPS 绑定，不是硬编码快捷键；手机指针也没有固定中键绑定。
避免将指针切换与左键、FPS 开关或其他动作绑定在同一按键。
键盘映射继续有效，指针模式暂停开火映射及包含鼠标绑定的单点映射。

## 实现与限制

- 手机箭头由 `phone-pointer/MaskPointer.java` 使用 Android SurfaceControl 绘制。
  它只显示箭头，不接收点击或占用窗口焦点；点击由 scrcpy 控制连接注入触摸。
- 辅助服务由 ADB shell 的 `app_process` 启动，无需安装 APK。
  每次手机重启后需要重新启动服务；本机 TCP 27821 转发至手机抽象 socket。
- 手机服务当前仅支持单个客户端、主设备、默认显示层；目前实测 Windows + Android 15。
  不同 Android 版本的隐藏 API 可能不同，尚未全面验证。
- 短点击按事件顺序保留，触摸按下至少 50 毫秒；失焦或退出时立即清理触摸。
- FPS 空闲 150 毫秒抬起触点，下次移动重新起滑；触摸按配置分辨率量化，减少坐标台阶。
- Windows 最小化坐标不再写入配置，进入 FPS 及恢复焦点时重新捕获鼠标。
- `SCRCPY_MASK_DATA_DIR` 可指定独立数据目录，适合原版、测试版分别运行。

## 构建手机指针（Windows）

先按上游 [构建说明](build-help.md) 准备 Rust、FFmpeg、ADB 和前端依赖。
另需 JDK 17 和 Google R8/D8。已验证 R8 8.3.37：

下载地址：https://storage.googleapis.com/r8-releases/raw/8.3.37/r8.jar

SHA-256：`900dfbc649519969fc5a4c7520d6b7355338e565fa1249874e0190b8d61b1199`

```powershell
./scripts/build-phone-pointer.ps1 -JavaHome 'C:\path\to\jdk-17' -R8Jar 'C:\path\to\r8.jar'
pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend build
cargo build --locked --release
./scripts/start-phone-pointer.ps1
```

构建得到 `assets/mask-pointer.jar`，现有 Windows 打包脚本会把 assets 一起打包。
运行辅助脚本可通过 `-Adb`、`-Jar`、`-DeviceId` 指定路径或设备。
它使用固定端口 27821，与源码中的指针连接端口对应。

独立数据目录示例：

```powershell
$env:SCRCPY_MASK_DATA_DIR = Join-Path $PWD 'local\custom-data'
& ./target/release/scrcpy-mask.exe
```

辅助服务与桌面程序需要使用同一台 ADB 设备；桌面连接和映射配置按上游操作。

## 已完成验证

- `cargo test --locked --release --lib`：47 项测试通过。
- `cargo build --locked --release` 和前端 TypeScript/Vite 构建通过。
- 手机箭头显示、辅助服务自动启动、配置保存及重新加载已验证。
- 新增测试覆盖短点击、快速双击、切换后立即点击、拖动坐标一致、失焦清理和起点换算。

游戏中的手感、不同游戏的触摸兼容性仍需实际试用。个人键位、设备序列号、日志、
第三方二进制及本机构建目录不包含在提交中。
