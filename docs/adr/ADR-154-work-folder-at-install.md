# ADR-154: The work folder is chosen at install; no setup screen

- **Status:** Accepted
- **Date:** 2026-11-01 (Asia/Saigon)
- **Asked by the owner:** "lúc người dùng chạy file cài đặt thì nên cho họ chọn nơi làm việc của agenium luôn, còn cài
  đặt xong, họ mở app thì vào thẳng giao diện làm việc chính luôn, ngôn ngữ thì họ tự đổi trong màn hình chính sau";
  then, of the options given: drop the MSI, and on Linux and macOS ask once at the first start.

## Context

The first start showed a setup screen: a language and a "Create the root" button that set the root up where the app
had already resolved it (`AGENIUM_HOME`, the bootstrap file, else `~/.agenium`), without saying where or letting the
user choose. Only the `agenium setup --root` command could put the root elsewhere, for example on drive D:. The
language is changed at any time with the VI/EN switch, which already saves it to the config.

## Decision

1. **Windows asks in the installer.** The NSIS installer has a page after the install directory, "Agenium's work
   folder": the folder in use (from `%APPDATA%\agenium\bootstrap.toml`) or `%USERPROFILE%\.agenium`, a Browse button
   (a folder that is not Agenium's gets an `Agenium` folder inside it: `D:\` becomes `D:\Agenium`), and a note that
   updates and uninstalling never touch it and that choosing another folder moves nothing. The installer writes the
   bootstrap file; a silent install takes `/DATA=<folder>` and otherwise keeps the folder chosen before. The page lives
   in `apps/desktop/installer/windows/hooks.nsh`; Tauri's NSIS template is copied as `installer.nsi` with one line
   added, because the bundler's hooks cannot add a page.
2. **The MSI is dropped.** Its dialogs would need a WiX template of their own; the NSIS installer is the Windows one.
3. **Linux and macOS ask once at the first start.** Their packages (`.deb`, AppImage, `.dmg`) have no step to ask in.
   When nothing names the root (no `AGENIUM_HOME`, no bootstrap file) and `~/.agenium` is not set up, the app first
   shows "Agenium's work folder" with the default, a Choose… button (the system's folder dialog,
   `tauri-plugin-dialog`, with the same `Agenium` rule) and Start; nothing is written before the answer, so the
   embedding model is put straight into the chosen folder. A folder other than the default is remembered in the
   bootstrap file.
4. **No setup screen.** Once the embedding model is in place, a root without `config.toml` is set up on its own, in
   the system's language (Vietnamese or English), and opened: the app opens on the overview. The `setup` command is
   gone; `agenium setup --root` stays for the CLI.

## Consequences

- `apps/desktop/installer/windows/{installer.nsi,hooks.nsh}`; `tauri.bundle.json` (no `msi`; `template`,
  `installerHooks`).
- Desktop: `api/first_run.rs` (`set_up_root`, `pick_work_folder`, `choose_work_folder`); `Desktop`'s root can be set
  once (`root()`, `set_root`, `asking_root`); `model::ensure` sets the root up before opening it; `main.rs` decides
  whether a first start asks; dependencies `tauri-plugin-dialog`, `sys-locale`.
- UI: `views/WorkFolder.tsx`; the setup screen is gone from `Admin.tsx`; the model screen says the folder is opening;
  strings `setup.*`. The UI smoke test expects the app to open on its own.
