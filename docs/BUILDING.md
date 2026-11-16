# Build EDCoPilot VoiceBridge from source

This document tells a developer how to build and test VoiceBridge on Windows.
It is for Windows 10 or Windows 11 on an x64 computer.

The repository has two programs:

- `voicebridge` is the console program (crate in `app/`).
- `voicebridge-ui` is the window program (crate in `ui/`, web part in `ui/web`).
  The installer contains this program.

Other documents:

- `docs/USAGE.md` tells a user how to use the programs.
- [`docs/DEVELOPMENT.md`](DEVELOPMENT.md) tells you how to run the live tests
  against a real EDCoPilot installation. This document does not describe them.

## Prerequisites

| Tool | Version | Note |
| --- | --- | --- |
| Visual Studio Build Tools | 2022 or later | Workload "Desktop development with C++" (MSVC compiler and Windows SDK). |
| Rust (rustup) | 1.97.1 | Target `x86_64-pc-windows-msvc`. Components `clippy` and `rustfmt`. |
| Node.js | 24 | npm 11 comes with Node.js 24. |
| Git | any | To get the source. |
| WebView2 runtime | any | Windows 11 has it. The window program needs it. |

You do not need CMake. The build does not compile C or C++ code. The Tauri
CLI comes from `ui/web/package-lock.json` (`@tauri-apps/cli` 2.12.4). You do
not install it globally.

The CI workflow `.github/workflows/ci.yml` uses the same versions. When you
change a version, change it in this document and in both workflow files.

### Install the Rust toolchain

1. Install rustup from <https://rustup.rs>.
2. Open a new terminal.
3. Run `rustup toolchain install 1.97.1 --component clippy,rustfmt`.
4. Go to the repository folder.
5. Run `rustup override set 1.97.1`.

The override applies to this folder only. Rustup keeps it in its own settings,
not in the repository.

## The sherpa-onnx library

The speech engine uses the crate `sherpa-onnx` 2.0.1. Its build script links
the prebuilt static libraries of sherpa-onnx and ONNX Runtime into the
programs. The programs need no sherpa-onnx DLL.

The libraries come from one archive of the official sherpa-onnx release:

| Item | Value |
| --- | --- |
| URL | `https://github.com/k2-fsa/sherpa-onnx/releases/download/v2.0.1/sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib.tar.bz2` |
| Size | 122,863,596 bytes (about 1.1 GB unpacked) |
| SHA-256 | `23e7b986b197eda0bbc47ff3d7d5c87afd6ef2b9077214ba26b0f1ec52ef0e66` |

There are two methods to get the libraries.

### Method 1: the automatic download

Do nothing. When `SHERPA_ONNX_LIB_DIR` is not set, the build script downloads
the archive at the first build. It puts the archive and the unpacked files
in `<target folder>/sherpa-onnx-prebuilt/`.

The build script does not check a hash. Each new target folder gets a new
download.

### Method 2: a checked download (recommended)

Do these steps in PowerShell, in the repository folder:

1. Make the folder for the archive:

   ```powershell
   New-Item -ItemType Directory -Force target\sherpa-onnx-prebuilt
   ```

2. Download the archive:

   ```powershell
   curl.exe -L -o target\sherpa-onnx-prebuilt\sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib.tar.bz2 https://github.com/k2-fsa/sherpa-onnx/releases/download/v2.0.1/sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib.tar.bz2
   ```

3. Show the SHA-256 of the archive:

   ```powershell
   (Get-FileHash -Algorithm SHA256 target\sherpa-onnx-prebuilt\sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib.tar.bz2).Hash
   ```

4. Compare the value with the SHA-256 in the table above. The case of the
   letters is not important.
5. If the values are different, delete the archive and stop.
6. Unpack the archive:

   ```powershell
   tar.exe -xjf target\sherpa-onnx-prebuilt\sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib.tar.bz2 -C target\sherpa-onnx-prebuilt
   ```

7. Set `SHERPA_ONNX_LIB_DIR` to the absolute path of the `lib` folder:

   ```powershell
   $env:SHERPA_ONNX_LIB_DIR = "$PWD\target\sherpa-onnx-prebuilt\sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib\lib"
   ```

The variable applies to the current terminal only. Set it again in each new
terminal, or set it as a user environment variable.

The unpacked files are in the same place as the files of Method 1. Thus a
build without the variable uses them too, and it does not download again.

## Build and run the console program

1. Build the program:

   ```powershell
   cargo build -p voicebridge
   ```

2. Show the commands of the program:

   ```powershell
   cargo run -p voicebridge -- --help
   ```

3. Show the microphones:

   ```powershell
   cargo run -p voicebridge -- devices
   ```

To configure the console program, read `docs/USAGE.md`.

## Run the tests

### The Rust tests

1. Check the format:

   ```powershell
   cargo fmt --all --check
   ```

2. Run Clippy:

   ```powershell
   cargo clippy --workspace --all-targets -- -D warnings
   ```

3. Build the two programs without the test features:

   ```powershell
   cargo build -p voicebridge -p voicebridge-ui
   ```

4. Run the tests:

   ```powershell
   cargo test --workspace
   ```

Step 3 is necessary. The test build enables the feature `test-support` of the
core crate. Thus `cargo test` can pass when the normal build fails.

The tests need no microphone, no internet access, and no EDCoPilot installation. The
tests that need these things have the attribute `#[ignore]`, and `cargo test`
skips them. Do not run all ignored tests at the
same time. Some of them download a speech model or open the microphone.

### The tests of the web part

Do these steps in the folder `ui/web`:

1. Install the dependencies from the lockfile:

   ```powershell
   npm ci
   ```

2. Check the types:

   ```powershell
   npm run check
   ```

3. Run the unit tests:

   ```powershell
   npm test
   ```

## Build the window program for development

1. Do the steps of "The tests of the web part" until `npm ci`.
2. Set `VOICEBRIDGE_DEV_DATA_DIR` to an absolute scratch folder:

   ```powershell
   $env:VOICEBRIDGE_DEV_DATA_DIR = "$env:TEMP\voicebridge-dev"
   ```

3. In the folder `ui/web`, start the development build:

   ```powershell
   npm run tauri dev
   ```

A development build never uses the data folder of the user
(`%APPDATA%\VoiceBridge`). Without `VOICEBRIDGE_DEV_DATA_DIR` or
`--config`, it uses no configuration.

`cargo build -p voicebridge-ui` compiles the debug program too. That program
loads the web part from the development server (`http://127.0.0.1:1420`).
Thus it shows a page only while `npm run tauri dev` or `npm run dev` runs.

The development build and the installed program have the same identifier.
Thus they cannot run at the same time.

## Build the installer

1. Do the steps of "The tests of the web part" until `npm ci`.
2. Do Method 2 of "The sherpa-onnx library". Make sure that
   `SHERPA_ONNX_LIB_DIR` has a value in this terminal.
3. Optional: set a separate target folder for the release build:

   ```powershell
   $env:CARGO_TARGET_DIR = "$PWD\target\installer"
   ```

4. In the folder `ui/web`, build the installer:

   ```powershell
   npm run tauri build
   ```

The build makes the web part, the release program, and the NSIS installer.
The installer is in `<target folder>\release\bundle\nsis\`. Its name is
`EDCoPilot VoiceBridge_<version>_x64-setup.exe`. The target folder is
`target` when `CARGO_TARGET_DIR` is not set.

The build needs no signing key. `ui/tauri.conf.json` sets
`bundle.createUpdaterArtifacts` to `false`: the build makes no updater
signature. With `true`, `tauri build` stops without the key (error "A public
key has been found, but no private key"), and the key would be in the
environment of each build script.

### Sign an installer for the updater

Only the owner of the key does this, and only for a test. The release
workflow signs the installers of the releases. Do these steps in the
repository folder, after the build:

1. Set the path of the key file and the empty password:

   ```powershell
   $env:TAURI_SIGNING_PRIVATE_KEY_PATH = "$PWD\private\updater\voicebridge.key"
   $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
   ```

2. Sign the installer. Use the version of the program:

   ```powershell
   node ui\web\node_modules\@tauri-apps\cli\tauri.js signer sign --app-version 0.2.0 "target\release\bundle\nsis\EDCoPilot VoiceBridge_0.2.0_x64-setup.exe"
   ```

3. Remove the two variables:

   ```powershell
   Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PATH, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
   ```

The command writes `<installer>.sig` next to the installer. `--app-version`
puts the version into the signed comment. The program refuses a signature
without the version or with another version. `tauri signer sign` reads
`TAURI_SIGNING_PRIVATE_KEY` as the key itself, never as a path.

At the first build, the Tauri bundler downloads NSIS and
`nsis_tauri_utils.dll` into `%LOCALAPPDATA%\tauri`. The next builds use this
copy.

The installer installs for the current Windows user only. It needs no
administrator rights. When WebView2 is not on the computer, the installer
downloads the WebView2 bootstrapper from Microsoft and starts it.

## Notes

### The installer has no code signature

The project does not sign the installer. Windows SmartScreen can show
"Windows protected your PC" when a user starts it. The user clicks
"More info" and then "Run anyway". The release workflow publishes a SHA-256
for each installer.

### Debug and release builds

| Item | Debug build | Release build |
| --- | --- | --- |
| Command | `cargo build`, `npm run tauri dev` | `npm run tauri build`, `cargo build --release` |
| Web part | From the development server | Inside the program |
| Data folder | `VOICEBRIDGE_DEV_DATA_DIR` or `--config` only | `%APPDATA%\VoiceBridge` |

### The static C runtime

The sherpa-onnx libraries use the static C runtime (`/MT`). Thus
`.cargo/config.toml` sets `-C target-feature=+crt-static` for the Rust code.
Do not set the environment variable `RUSTFLAGS`. It replaces the flags of
`.cargo/config.toml`, and the linker then gives the warning `LNK4098`.

### Environment variables

| Variable | Use |
| --- | --- |
| `SHERPA_ONNX_LIB_DIR` | The folder with the unpacked sherpa-onnx `.lib` files. The build script then downloads nothing. |
| `SHERPA_ONNX_ARCHIVE_DIR` | A folder with the archive. The build script copies the archive from there and does not download it. |
| `CARGO_TARGET_DIR` | A separate target folder, for example for the installer build. |
| `VOICEBRIDGE_DEV_DATA_DIR` | The data folder of a development build of the window program. It must be an absolute path. |
| `TAURI_SIGNING_PRIVATE_KEY_PATH` | The path of the updater signing key, for `tauri signer sign`. See "Sign an installer for the updater". |
| `TAURI_SIGNING_PRIVATE_KEY` | The updater signing key itself. Only the release workflow sets it, in its signing step. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The password of the signing key. The key of the project has none: set an empty value. |
| `RUSTFLAGS` | Do not set it. See "The static C runtime". |

## Continuous integration and releases

- `.github/workflows/ci.yml` runs for each pull request and each push to the
  default branch. It does the steps of "Run the tests".
- `.github/workflows/release.yml` runs for each tag `v*`. It builds and signs
  the installer, writes `latest.json`, and makes a draft release. The owner
  publishes the draft.

Both workflows download the sherpa-onnx archive and check its SHA-256 (Method
2).

To make a release:

1. Set the new version in `ui/tauri.conf.json`.
2. Set the same version in each `Cargo.toml` of the workspace.
3. Run `cargo update --workspace`. This command writes the new versions into
   `Cargo.lock`.
4. In the folder `ui/web`, run `npm version 0.2.0 --no-git-tag-version`. Use
   the new version. This command changes `package.json` and
   `package-lock.json`.
5. In `CHANGELOG.md`, add a section `## 0.2.0` above the older sections.
   Write the changes of this version as a short list.
6. Commit the changes.
7. Build the installer and do the checks of [CHECKLIST.md](CHECKLIST.md).
8. Make the tag, for example `git tag v0.2.0`.
9. Push the tag.
10. Wait for the workflow. Open the draft release on GitHub.
11. Download the installer of the draft. Compare its SHA-256 with
    `SHA256SUMS.txt`. Install it on a test computer and do a short check.
12. Do the steps N1 and N2 of [CHECKLIST.md](CHECKLIST.md).
13. Publish the draft.
14. Do the steps N3 to N12 of [CHECKLIST.md](CHECKLIST.md) with an older
    installed version.

The workflow stops when the tag and a version are different. It also stops
when `CHANGELOG.md` has no section for the version. The text of that section
becomes the "Changes" part of the release notes and the `notes` of
`latest.json`.

### The update files of a release

The installed programs read
`https://github.com/Superbrain8/edcopilot-voicebridge/releases/latest/download/latest.json`.
GitHub gives this address only for the newest published release. A draft or a
pre-release does not reach the users. Thus the order is: draft, then the test
of the installer, then publish. After you publish, each installed program
finds the new version at its next check.

The release workflow does these steps for the update:

1. The job "build" builds the installer. It has no secret. Thus npm, the
   build scripts of the crates, the bundler, and NSIS never see the key.
2. Only for a tag, the job "sign" signs the finished installer with
   `tauri signer sign --app-version <version>`. The job runs in the GitHub
   environment `release` and reads the secrets `TAURI_SIGNING_PRIVATE_KEY`
   and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (optional) from it. Only one
   step has the key in its environment. In that step, two processes see the
   key: the PowerShell of the step and `node.exe`, which runs the Tauri CLI
   in its own process. The job installs the CLI with `npm ci
   --ignore-scripts` before that step. A tag without the secret stops with an
   error. A dry run (workflow_dispatch) never signs.
3. The release job writes `latest.json` with `jq`: the version without "v",
   the text of the CHANGELOG section, the time, the content of the `.sig`
   file, and the address of the installer asset of the tag.
4. The job checks `latest.json`: the file is JSON, its version is the version
   of the tag, the address is the installer asset, and the signature is not
   empty.
5. The job checks the signature with `minisign` and the public key of
   `ui/tauri.conf.json`. The signed comment must have the version of the tag.
6. The draft gets the installer, `<installer>.sig`, `SHA256SUMS.txt`, and
   `latest.json`. `SHA256SUMS.txt` has only the installer.

Do not rename or replace the installer asset after the workflow. The address
in `latest.json` names the asset, and the signature covers its bytes.

The secret `TAURI_SIGNING_PRIVATE_KEY` holds the content of the private key
file. See "The signing key" in [DEVELOPMENT.md](DEVELOPMENT.md) for the
environment `release`.

To test the release build without a release, start the workflow "Release" by
hand (workflow_dispatch). The installer is then a workflow artifact only.
