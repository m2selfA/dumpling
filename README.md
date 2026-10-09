# Dumpling

打开就能用的本机服务转发。连接层嵌在程序里，不需要另外安装 dumbpipe。ticket 仍与 dumbpipe 的 `listen-tcp` / `connect-tcp` 兼容。GUI binary 只在 Windows/macOS 构建；Linux 构建路径只提供 CLI，待 wind-ui 的 Linux 支持满足本项目需求后再恢复 GUI。

- 主界面通过「连接 / 共享」切换，只显示当前模式。
- 连接：粘贴 ticket，回车或点击「连接」。程序会自动选择 `127.0.0.1` 上的空闲端口转发到对方，然后打开浏览器并收到托盘。
- 共享：切到「共享」，填要共享的本机服务端口（默认 `8080`），点击「共享」。成功后自动回到「连接」页，复制生成的 ticket。

```bash
cargo run --features gui --bin dumpling
```

### CLI 共享器

CLI 版本是独立的 console binary，适合交给 systemd、NSSM、WinSW 等服务管理器运行：

```bash
# 默认共享 127.0.0.1:8080
cargo run --bin dumpling-cli -- share --port 8080

# 生成 release 版后直接运行
./target/release/dumpling-cli share --port 8080 --ticket-file ticket.txt
```

启动后第一行 stdout 是 ticket，进程会持续运行；按 `Ctrl+C` 停止。`--ticket-file` 可让服务管理器或其他程序读取当前 ticket。`connect` 子命令预留给后续连接功能。

### Release 体积策略

Release 使用 size-oriented profile：`opt-level = "z"`、Thin LTO、单 codegen unit 和符号剥离；保留 panic unwind，避免改变 GUI/服务的故障语义。Windows GUI 仅启用 windui 的 `d2d` feature，不包含未使用的 SVG/resvg。

### CI 与 release

普通 branch push 和 pull request 只运行检查；`v*` tag 会构建 Windows/macOS GUI 与 CLI，以及 glibc 2.17 兼容的 Linux CLI，并创建 GitHub Release。GitHub-hosted runner 当前不提供 Windows 10 label，Windows 检查使用固定的 `windows-2022`；若需要真实 Windows 10 验证，可将 repository variable `DUMPLING_WINDOWS_RUNNER` 设置为 self-hosted runner 的 `win10` label。

图标在 `assets/`：`dumpling-32.png.b64` 供窗口、任务栏和托盘解码后使用，`dumpling.ico.b64` 在 Windows 构建时嵌入 exe。
