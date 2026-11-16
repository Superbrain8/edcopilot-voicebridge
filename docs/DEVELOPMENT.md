# Development notes

This document is for a developer who changes the code. It gives the
architecture, the rules that the code obeys, the live tests, the known
limits, and the open work.

- `docs/BUILDING.md` tells you how to install the tools, build the programs,
  run the normal tests, and make the installer.
- `docs/USAGE.md` is the user guide.
- `docs/PROFILE_FORMAT.md` describes the EDCoPilot profile and the map
  builder.

## What the program does

VoiceBridge gives EDCoPilot voice control without VoiceAttack. The data flow
is:

1. The microphone gives audio. The audio crate changes it to 16 kHz mono and
   cuts one utterance (push-to-talk or always-listening).
2. A speech-to-text engine changes the utterance to text. The local engine
   is Parakeet with sherpa-onnx. The cloud engine is a service with an
   OpenAI-compatible interface.
3. The matcher finds the command whose phrase fits the text and gives a
   confidence value.
4. The matcher fills the token template of the command (names, numbers,
   constants, the prefix of a composite command).
5. The writer writes the token to `EDCoPilot.request.txt`. EDCoPilot reads
   this file approximately each 500 ms. The writer waits until EDCoPilot
   reads the token.

The program builds the commands from the EDCoPilot profile of the user at
each load. The repository has no command data from EDCoPilot.

## The crates

| Crate | Folder | Function |
|---|---|---|
| `edcopilot-voicebridge-core` | `core/` | Text to token to request file. No microphone, no network, no user interface. |
| `edcopilot-voicebridge-stt` | `stt/` | The engine interface `SttEngine`. The local Parakeet engine (feature `local`) and the cloud engine (feature `cloud`). WAV helpers. |
| `edcopilot-voicebridge-audio` | `audio/` | Microphone capture, the converter to 16 kHz mono, the segmenter, and the talk button. |
| `voicebridge` | `app/` | The configuration, the wake word, the session, and the console program. The session loop with its events (`runtime.rs`), the configuration edits (`config_edit.rs`), and the word tables as rows (`tables.rs`). |
| `voicebridge-ui` | `ui/` | The window program (Tauri 2). The commands for the web part (`commands.rs`, `dto.rs`), the key store (`secrets.rs`), the model download (`model_download.rs`), the data folder and the first start (`paths.rs`), and the tray (`tray.rs`). |

The web part of the window program is in `ui/web`. It is Svelte 5 with
TypeScript and Vite. `ui/web/src/routes/` has one file for each screen.
`ui/web/src/lib/` has the stores and `api.ts`, the typed calls into the Rust
part. Vitest runs its tests.

The modules of the core crate:

| Module | Function |
|---|---|
| `profile` | Builds the commands and the profile variables from the bytes of `EDCoPilot-Profile.vap`, with a report of the left-out commands. |
| `command_map` | The record of one command. |
| `runtime_grammar` | Puts the grammar from `EDCoPilot.VoiceCommands.ini` into the phrases. |
| `grammar` | Parses VoiceAttack grammar into a tree. |
| `dynamic_vocab` | Loads the names in `Dynamic var files` and monitors the folder. |
| `aliases` | Loads phrases in other languages from `aliases/<language>.toml`. |
| `matcher` | Matches text to one command token and gives a confidence value. It joins a composite prefix and suffix into one token. |
| `writer` | Writes the token to the request file and waits until EDCoPilot reads it. |
| `numbers` | Changes number words to digits, in English and German. |
| `sound_alikes` | Loads the sound-alike words and makes the text variants. |
| `synonyms` | Loads the phrase synonyms and makes the text variants. |
| `fillers` | Loads the filler words and makes the variants without them. |
| `pipeline` | `match_text`, `send_match`, `handle_text`, and `set_dynamic_vocab`. |

`match_text` has five steps: the text with its number variants, the synonym
variants, the sound-alike variants of each, and the filler step. It goes to
the next step only when no candidate reached the threshold. The filler step
is off in `always_listening` mode.

## Rules that the code obeys

Do not break these rules. Many tests check them.

- **A wrong match is worse than a miss.** The matcher gives no result when
  two different tokens are equally good, or when a placeholder has no
  value. The map builder leaves out each command that it does not fully
  understand.
- The matcher accepts a phrase only when the phrase uses all the spoken
  words.
- A slot file such as `EDCoPilotSystem1.txt` has the system name first. The
  other entries are phonetic spellings and stations. The token always gets
  the first entry.
- A variant step (synonym, sound-alike, filler) may confirm a command. It
  never replaces a literal match or a tie. The second pass of the hotwords
  may confirm a command, never redirect it.
- The writer does not replace a token that EDCoPilot did not read.
- The writer waits until EDCoPilot reads the token. If EDCoPilot does not
  read it in 2 seconds, the writer removes the token and gives the error
  `NotConsumed`. Thus an old command cannot run when EDCoPilot starts later.
- **The API key never goes into a log line, an error message, a `Debug`
  output, or the configuration file.** The configuration holds only the
  name of an environment variable. The window program keeps the key in the
  Windows Credential Manager under this name.
- The repository holds no EDCoPilot data. Tests use invented data. Real
  data stays in `private/` (see "Live tests").
- **The program installs only an update with a correct signature, and only
  after a click.** No code path starts an installer without the signature
  check of the updater plugin. A session start and an update install never
  run at the same time (see "The updater").

## The updater

The window program uses the Tauri updater plugin from the Rust side only
(`ui/src/updater.rs`). The capability file gives the page no updater
permission. The page calls the commands of `commands::ipc`.

- `ui/tauri.conf.json` has the update address, the public key, and
  `requireSignedVersion`. No setting of the user changes them. A test
  (`test_the_plugin_configuration_is_safe`) compares them with the expected
  values.
- The plugin checks the signature of the installer with the public key. The
  installer itself has no Windows code signature. Thus the updater key is the
  only protection against a changed installer.
- The capture lock is the gate between a session start and an install. See
  the comment at the top of `ui/src/state.rs`.
- Just before the installer starts, the program stops the session and hides
  the tray icon. It does not call `cleanup_before_exit`. If the installer
  does not start, the program shows the icon and the window again.
- The window program has no log file. The Diagnostics screen shows the
  result of the last check.
- The tests use a throwaway key pair. `ui/fixtures/updater` has its public key
  and the files that it signed. The tests never use the key of the project.

### The signing key

The private key of the project signs each installer. The key has no
password. The owner keeps a copy in `private/updater/voicebridge.key`. Git
ignores the folder `private/`. Never commit the key, never print it in a log,
and never send it in a message.

Make a backup of the key in a safe place outside the computer, for example a
password manager.

#### The environment `release`

The release workflow signs in the job "sign" only. That job runs only for a
tag, in the GitHub environment `release`. Set it up one time:

1. On GitHub, open the repository settings. Open "Environments".
2. Make the environment `release`.
3. Under "Deployment branches and tags", select "Selected branches and tags".
   Add the rule for the tags `v*`. Then a run of another branch or tag cannot
   use the secrets.
4. Add the environment secret `TAURI_SIGNING_PRIVATE_KEY`. Its value is the
   content of `private/updater/voicebridge.key`.
5. Optional: add the environment secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
   The key of the project has no password: an absent secret gives an empty
   password.
6. Remove a repository secret `TAURI_SIGNING_PRIVATE_KEY`, if one exists.
   Only the environment must hold the key.
7. Open "Rules", then "Rulesets". Make a tag ruleset for `v*`. Let only the
   owner make, move, or delete these tags.

Without the environment, GitHub makes it at the first tag run, without
rules and without secrets. The job "sign" then stops with an error.

#### A new key (rotation)

The installed programs know only the public key of their version. Thus the
change to a new key needs a bridge release:

1. In the folder `ui/web`, make a new key pair:

   ```powershell
   npm run tauri signer generate -- -w ..\private\updater\voicebridge-new.key
   ```

2. Put the new public key (the content of the `.pub` file) into
   `plugins.updater.pubkey` of `ui/tauri.conf.json` and into the test
   `test_the_plugin_configuration_is_safe`.
3. Release this version (the bridge release) signed with the OLD key. The
   installed programs know only the old key. Thus they can install the
   bridge release, and then they know the new key.
4. Keep the bridge release as the newest release for 60 days. `latest.json`
   names only the newest release. A program that is still on a version before
   the bridge release can update only while the bridge release is the newest
   release.
5. After the 60 days, replace the secret `TAURI_SIGNING_PRIVATE_KEY` of the
   environment `release` with the new key. The next releases use the new key.

A program that is still on a version before the bridge release when the
first release of the new key becomes the newest release can never verify an
update. Its user must install the next version by hand from the Releases
page.

#### A leaked key

Nobody can revoke a leaked key from a distance. The installed programs trust
it for ever. A person with the key can make an installer that these programs
accept. That person must also change the files of the GitHub release, or the
update address, to reach the users. Do these steps at once:

1. Make a bridge release with a new key, as in "A new key (rotation)". Sign
   it with the old key: it is the only key that the installed programs know.
2. Put a notice on the GitHub page of the project. Tell the users to update
   to the bridge release, or to install it by hand.

#### A lost key

Without the key, no update of the installed programs is possible. Make a new
key pair and put its public key into the next version. Each user must install
that version by hand from the Releases page. From then on, the new key signs
the releases.

## Live tests

The normal tests (`cargo test --workspace`, see `docs/BUILDING.md`) need no
EDCoPilot installation, no model, no microphone, and no private file. The
tests that need one of these things have the attribute `#[ignore]`.

Run an ignored test only by its file or name. Some ignored tests download
the model (about 487 MB) or open the microphone. Do not run all ignored
tests of the workspace at the same time.

### Private files

The folder `private/` in the repository folder holds test data that must not
be public. `.gitignore` names it. Make the folder yourself.

| File | Content | Used by |
|---|---|---|
| `private/live_probes.tsv` | Real phrases and the expected tokens | `core/tests/live_probes.rs`, `app/tests/live_probes.rs` |
| `private/built_map_<version>.json` | A snapshot of the build of the real profile | `core/tests/profile_live.rs` |
| `private/stt_fixtures/*.wav` | Short speech clips (16 kHz mono WAV) | `stt/tests/parakeet_live.rs`, the Parakeet tests of `stt/src/parakeet.rs`, two ignored tests of `ui` |

The format of the probe file is in the module comment of
`core/tests/live_probes.rs` and `app/tests/live_probes.rs`. The speech tests
expect these clips:

| File | What the voice says |
|---|---|
| `en_set_course_home.wav` | An English command with the words "course" and "home" |
| `en_music_volume_55.wav` | An English command with the word "volume" and a number |
| `de_kurs_nach_hause.wav` | A German phrase with the words "Kurs" and "Hause" |
| `en_navigate_to_sol.wav` | "navigate to Sol". Greedy search hears "Seoul". |

When a clip or the probe file is absent at the default path, the test writes
a note to stderr and passes.

### Environment variables of the tests

| Variable | Meaning |
|---|---|
| `VOICEBRIDGE_EDCOPILOT_DIR` | The EDCoPilot folder, for example `C:\EDCoPilot`. The live tests read it and write nothing there. |
| `VOICEBRIDGE_LIVE_PROBES` | The probe file. Default `private/live_probes.tsv`. A relative path starts at the workspace folder. An explicit path that does not read makes the test fail. |
| `VOICEBRIDGE_MAP_SNAPSHOT` | The private snapshot of the build. A relative path starts at the repository folder. |
| `VOICEBRIDGE_MAP_SNAPSHOT_WRITE` | `1` writes the snapshot in place of the compare. The write refuses a path in the repository that is not under `private/`. |
| `VOICEBRIDGE_SHADOW_OUT` | A private file for the texts of the shadow test of `core/tests/live_probes.rs`. Put it in `private/`. |
| `VOICEBRIDGE_STT_FIXTURES` | The folder of the speech clips. Default `private/stt_fixtures`. A relative path starts at the workspace folder. |
| `VOICEBRIDGE_PARAKEET_DIR` | The folder of the Parakeet model. The Parakeet tests skip when it has no model. |
| `VOICEBRIDGE_GOLDEN_WRITE` | `1` writes the golden file of `core/tests/fixture_golden.rs` again. Read the difference in git before the commit. |
| `VOICEBRIDGE_UI_CONFIG` | A configuration file for the timing tests of `app/src/tables.rs` and `ui/src/commands.rs`. |
| `VOICEBRIDGE_DEV_DATA_DIR` | The data folder of a development build of the window program (see `docs/BUILDING.md`). |

### Examples

The live tests of the map builder:

```text
VOICEBRIDGE_EDCOPILOT_DIR=C:\EDCoPilot cargo test -p edcopilot-voicebridge-core --test profile_live -- --ignored --nocapture
VOICEBRIDGE_EDCOPILOT_DIR=C:\EDCoPilot cargo test -p edcopilot-voicebridge-core --test live_probes -- --ignored --nocapture
VOICEBRIDGE_EDCOPILOT_DIR=C:\EDCoPilot cargo test -p voicebridge --test live_probes -- --ignored --nocapture
```

The speech engine with the private clips:

```text
VOICEBRIDGE_PARAKEET_DIR=<model folder> cargo test -p edcopilot-voicebridge-stt --test parakeet_live -- --ignored --nocapture
```

The app probes run the console program with `text`, `commands`, and `check`
only. A test never uses `--send`.

### Send a test token to EDCoPilot

`core/examples/send_token.rs` writes one token to a request file and shows
if EDCoPilot read it. Use a token that does no harm, for example the token
of a command that opens a panel. `voicebridge commands --all` shows the
tokens of your installation.

For a test with EDCoPilot in operation:

- EDCoPilot needs 1 to 2 minutes to start.
- EDCoPilot stops after 15 minutes if it finds no Elite Dangerous session.

## Measured values

- EDCoPilot reads the request file approximately each 500 ms (measured: 34
  ms to 456 ms).
- The local engine needs about 0.065 seconds for one spoken command on the
  test computer (4 threads). The ignored test `stt/tests/parakeet_live.rs`
  measured 0.060 to 0.064 seconds for each of three clips.
- The map builder needs about 13 ms for the full profile in a release build
  (see `docs/PROFILE_FORMAT.md`).

## Known limits

- The joystick talk button is not proven while the game has the focus. The
  crate `gilrs` uses Windows Gaming Input, which can need a window with
  focus. Test it with the game in the foreground. If the button does not
  work, use a keyboard key.
- A queued utterance can go to EDCoPilot approximately 15 seconds late when
  the cloud service is slow. The program checks the age only before the
  transcription.
- If the default microphone of Windows changes during a session, the
  console program stops with exit code 1.
- The bookmark group commands cannot run. The phrases use `{TXT:Group<n>}`,
  but the files have the name `EDCoPilotBookmarkGrp<n>.txt`. Some number
  commands also need a bookmark group name or a preference value.
- The program does not read `EDCoPilot.VoiceCommandsCustom.ini`.
- Nobody has captured a composite token or a token with `{CMD}` from
  VoiceAttack. The tokens come from the actions in the profile.
- Commands that write two or more tokens are not in the map. The builder
  also leaves out the VoiceAttack maintenance commands.
- It is not known if EDCoPilot asks for a confirmation before the command
  that moves the fleet carrier. Think about this before you use
  always-listening mode.
- A single word with a small error can match at the 0.8 threshold. The
  setting `matching.exact_single_word` (default: true) rejects such a
  match.
- The engine cannot tell some near names apart (for example an engineer
  name and a star name that sound the same). Use the name that the engine
  hears correctly.

## Open work

- The live-capture experiments of `docs/PROFILE_FORMAT.md` (section "Open
  questions"). They decide the meaning of action field 28, the group slots,
  the composite tokens, and the `{CMD}` text.
- A code signature for the installer. Windows SmartScreen warns before the
  installer starts.
- The console program in the installer.
- A merge of the new entries of a shipped word table into the copies of the
  user. After an update, the copies stay as they are.
- A warning for an alias phrase that is very broad.
- A problem text for an alias file that does not parse. The program skips
  such a file without a message.
- The check of a word table does not show the problems of a broken synonym,
  sound-alike, or filler file.
- The alias loader and the hotword loader accept a Unicode format character
  (for example U+200B) in a file that a person edited by hand.
- One instance: a first program with administrator rights blocks a second
  start without them. A second start while the first program ends can end
  without a window.
- The development build and the installed program have the same identifier
  `local.voicebridge.ui`. They cannot run at the same time.
- The icons are placeholders.
- A pipeline chain from a sound-alike word to a synonym.
- A bounded wait for the stop of the session at exit.
- A three-way compare at the reload of the configuration.
- Refuse a query or a fragment in the `base_url` of the cloud engine.
