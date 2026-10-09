# Dumpling

A local TCP service forwarder that works out of the box. The connection layer is embedded in the application, so dumbpipe does not need to be installed separately. Tickets remain compatible with dumbpipe's `listen-tcp` / `connect-tcp`. The GUI binary is built only for Windows and macOS; the Linux build provides the CLI only until wind-ui supports the GUI on Linux.

- The main window switches between **Connect** and **Share** and shows only the active mode.
- **Connect:** paste a ticket, then press Enter or click **Connect**. Dumpling automatically selects an available port on `127.0.0.1`, forwards the remote service, opens it in a browser, and moves the window to the tray.
- **Share:** switch to **Share**, enter the local service port (default `8080`), and click **Share**. After startup, the UI returns to **Connect** with the generated ticket ready to copy.

```bash
cargo run --features gui --bin dumpling
```

### CLI sharer

The CLI is a standalone console binary suitable for service managers such as systemd, NSSM, and WinSW:

```bash
# Share 127.0.0.1:8080 by default
cargo run --bin dumpling-cli -- share --port 8080

# Run the release build directly
./target/release/dumpling-cli share --port 8080 --ticket-file ticket.txt
```

The first stdout line after startup is the ticket, and the process keeps running until `Ctrl+C`. `--ticket-file` lets a service manager or another process read the current ticket. The `connect` subcommand is reserved for a future connection workflow.

### Release size strategy

The release profile is size-oriented: `opt-level = "z"`, Thin LTO, one codegen unit, and symbol stripping. Panic unwinding is retained so GUI and service failure semantics are not changed. The Windows GUI enables only windui's `d2d` feature and does not include the unused SVG/resvg stack.

### CI and releases

Branch pushes and pull requests run checks only. A `v*` tag builds the Windows and macOS GUI/CLI packages, builds the Linux CLI against glibc 2.17 compatibility, and creates a GitHub Release. GitHub-hosted runners currently do not provide a Windows 10 label; Windows checks use `windows-2022` by default. For exact Windows 10 validation, set the repository variable `DUMPLING_WINDOWS_RUNNER` to the label of a self-hosted `win10` runner.

Icons are stored in `assets/`: `dumpling-32.png.b64` is decoded for the window, taskbar, and tray, while `dumpling.ico.b64` is embedded into the Windows executable during the build.
