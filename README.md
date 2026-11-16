# EDCoPilot VoiceBridge

VoiceBridge lets you speak to EDCoPilot without VoiceAttack. EDCoPilot is the
companion program for Elite Dangerous. You speak a command into your
microphone. VoiceBridge changes your speech to text on your own PC. It finds
the matching voice command of your EDCoPilot installation. Then it gives that
command to EDCoPilot. VoiceBridge is free.

**Status:** This is an early version (0.1.0). One player made it for the
community and tested it on one PC only. Please tell us about problems and
ideas on the [Issues page](../../issues) of this repository.

## Is this for you?

VoiceBridge is a good fit if:

- You use EDCoPilot and you want to control it with your voice.
- You do not want to buy or set up VoiceAttack.
- The Windows speech recognition does not understand you well, for example
  because of your accent or your microphone.

VoiceBridge is not a fit if:

- You want voice commands that press keys in the game, or your own macros.
  VoiceBridge only talks to EDCoPilot. It never presses a key in the game.
- You need every EDCoPilot voice command. Some commands do not work yet (see
  below).
- You use Linux or macOS. VoiceBridge works only on Windows at this time.
  Most of its parts can work on other systems, but nobody built or tested
  it there. If you want to help with that, open an issue.

## What you need

- Windows 10 or Windows 11, 64 bit.
- EDCoPilot, installed. Start EDCoPilot one time before you use VoiceBridge.
  This makes the command file that VoiceBridge reads. VoiceAttack is not
  necessary.
- A microphone.
- About 1.2 GB of free disk space for the speech model download. After the
  download, the model uses about 670 MB.

## Install and first start

1. Open the [Releases page](../../releases) of this repository.
2. Download the installer file. Its name ends with `_x64-setup.exe`.
3. Start the file. The installer installs VoiceBridge for your Windows user
   only. It does not need administrator rights.
4. If Windows SmartScreen shows "Windows protected your PC", click "More info",
   then click "Run anyway". The warning shows because the program has no
   code signature. Do this only if you got the file from the Releases
   page of this repository.
5. Start EDCoPilot VoiceBridge. The Setup screen opens.
6. Beside "EDCoPilot folder", click "Browse" and select your EDCoPilot folder.
7. Click "Download the model". This downloads the speech model (about 487 MB)
   one time. Wait until the screen shows "The model is in ...".
8. Click "Save". Make sure that the screen shows "The check found no problem."
9. Open the Microphone screen. Select your microphone.
10. Click "Press to set", then press the key that you want to hold while you
    talk. Click "Save". The default key is F13, and most keyboards do not have
    it.
11. Start EDCoPilot and Elite Dangerous.
12. On the Status screen, click "Start as dry run". In a dry run, VoiceBridge
    shows what it heard, but it sends nothing to EDCoPilot.
13. Hold your talk key and say a command. Read the result row.
14. If the result is correct, click "Stop", then click "Start". Now your
    commands go to EDCoPilot.

A keyboard key is the safest talk key. A joystick button can fail to work
while the game window is active. For a joystick button, use the software of your
joystick. Make the button press your talk key.

When you close the window, VoiceBridge continues to run in the notification
area (the small icons near the clock). To end the program, right-click its
icon and select "Quit".

## How to talk to it

There are two modes. In push-to-talk mode, VoiceBridge listens only while you
hold your talk key. In always-listening mode, it listens all the time.

In always-listening mode, you can set a wake word, for example "copilot".
Then VoiceBridge obeys only a command that starts with that word. The wake
word is optional, but we recommend it. Without a wake word, other speech
near the microphone can start a command.

Say each command as EDCoPilot lists it. To find a command, open the Commands
screen and type a word in the filter. "Try a phrase" on that screen tests a
typed phrase and sends nothing.

VoiceBridge understands English commands. To use another language, you write
your own phrases for the commands on the Words screen. VoiceBridge includes a
small German example.

## VoiceBridge or VoiceAttack?

VoiceAttack is a mature, paid voice control program. EDCoPilot uses it for its
voice commands. VoiceBridge replaces only that one part. This table shows what
you gain and what you give up.

| | VoiceBridge | VoiceAttack with the EDCoPilot profile |
| --- | --- | --- |
| Price | Free. | Paid program. On its site, the price is 10 US dollars (November 2026). A free trial has one profile with up to 20 commands. |
| What it can control | EDCoPilot only. | EDCoPilot, key presses, mouse clicks, joystick buttons, and macros for any game or program. |
| Speech recognition | A modern speech model on your PC. It needs no voice training. | The Windows speech recognition. Its makers recommend voice training. Results can be less accurate with some accents or microphones. |
| Setup | Install, select the EDCoPilot folder, download the model, set the talk key. | Install, import the EDCoPilot profile, train the Windows speech recognition. |
| EDCoPilot commands | About 2,480 work. About 570 are left out (see below). The numbers depend on your EDCoPilot version. | All commands of the EDCoPilot profile. |
| Your own commands and macros | No. You can add other words for an EDCoPilot command, but no new actions. VoiceBridge does not read the custom commands file of EDCoPilot. | Yes, with many options, and with plugins. |
| Languages | English commands. Other languages need phrases that you write. | The languages of the Windows speech recognition, for example English, German, Spanish, and French. |
| When it is not sure | It does nothing. | It has its own confidence settings. See its help. |
| PC resources | The speech model uses your processor for a short time after each command. Recognition takes well under one second (about 0.065 seconds on the test PC). | No extra speech model to download. |
| Maturity and support | New. One person, one test PC. | Many years of use, a large community, and a forum. |

**Stay with VoiceAttack** if you want key presses or macros. Also stay if you
want your own voice commands for the game, or every EDCoPilot command.

**Try VoiceBridge** if you only want to talk to EDCoPilot. Also try it if you
do not want to buy or set up VoiceAttack. It can also help if the Windows
speech recognition does not understand you well.

You can have both programs on the same PC. But do not let both listen for
EDCoPilot commands at the same time. Then one spoken command can run two
times. We did not test this case.

## What VoiceBridge does not do

- A wrong command is worse than a missed command. When VoiceBridge is not
  sure, it does nothing. Thus some commands need a second try.
- Commands that need the name of a bookmark group do not work yet.
- VoiceBridge leaves out about 570 commands that it cannot send safely. The
  Diagnostics screen shows each of these commands with the reason.
- EDCoPilot must run. Without EDCoPilot, nothing happens.

## When something does not work

- **"No match".** Open the Commands screen and look for the command. Say it as
  the list shows it. Speak a bit slower, and keep the microphone near.
- **VoiceBridge hears the wrong words.** Look at the heard text in the result
  row. On the Words screen, add the wrong word as a sound-alike of the correct
  word. Then test it on the Commands screen.
- **A command is "left out".** VoiceBridge does not fully understand what the
  command does in EDCoPilot. Thus it never sends it. The Diagnostics screen
  shows the reason. Please report it on the Issues page if you need the
  command.
- **EDCoPilot had an update.** VoiceBridge reads the new commands of EDCoPilot
  at the next start of a session. On the Status screen, click "Stop", then
  click "Start". A large change in EDCoPilot can need a new version of
  VoiceBridge.
- **You need a log for a bug report.** On the Diagnostics screen, switch on
  "Utterance log". Click "Open folder" to find the file. Read
  [the user guide](docs/USAGE.md) for all problem messages.

## Privacy

- With the default speech engine, your voice stays on your PC. VoiceBridge
  sends no voice and no text to the internet.
- You can select a speech server instead (an OpenAI-compatible service). Then
  your voice goes to that server. The default setting does not use a speech server.
- VoiceBridge makes one download: the speech model, from the sherpa-onnx
  project on GitHub. The installer can also download Microsoft WebView2 from
  Microsoft, if your Windows does not have it.
- The window program checks for updates on github.com. It asks the releases
  page of the project for the newest version: about 10 seconds after the
  start, and then every 24 hours. It sends no data about you. To stop this
  check, clear "Check for updates at the start" on the Setup screen
  (`updates.check = false`). The button "Check for updates" on the
  Diagnostics screen still works.
- The program asks you before it installs an update. It never installs an
  update by itself, and never while a voice session runs.
- Your settings, word tables, model, and logs are in `%APPDATA%\VoiceBridge`.
  An uninstall keeps this folder. Delete it by hand to remove everything.
- A speech server can need an API key (a password for the service).
  VoiceBridge keeps this key in the Windows Credential Manager. It never writes
  the key to a file.

## For developers

- [The user guide](docs/USAGE.md) has every screen, setting, and message.
- [Building](docs/BUILDING.md) tells how to build the program and the
  installer. [Development](docs/DEVELOPMENT.md) tells how the code works.
- [The profile format](docs/PROFILE_FORMAT.md) tells how VoiceBridge reads the
  EDCoPilot profile.

## License and credits

VoiceBridge is free software under the GNU General Public License, version 3
or later. See [LICENSE](LICENSE).

VoiceBridge is not affiliated with EDCoPilot, VoiceAttack, or Frontier
Developments. Elite Dangerous, EDCoPilot, and VoiceAttack are names of their
owners. VoiceBridge ships no part of EDCoPilot. It reads the EDCoPilot files
on your PC.

The speech model is NVIDIA Parakeet TDT 0.6b v3, from the sherpa-onnx project,
under the CC-BY-4.0 license. See [NOTICE.md](NOTICE.md) for all credits.
