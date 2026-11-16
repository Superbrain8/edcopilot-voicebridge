# VoiceBridge user guide

EDCoPilot VoiceBridge gives EDCoPilot voice control without VoiceAttack.
EDCoPilot is the companion application for Elite Dangerous.

VoiceBridge works in four steps. It records your speech and changes it to text.
It matches the text to an EDCoPilot command. Then it writes the command token
to the EDCoPilot request file. The text comes from a local Parakeet model or from
an OpenAI-compatible cloud service.

The first rule of VoiceBridge is: "A wrong match is worse than a miss."

The program sends a command only when the match is clear. When two commands are
equally good, it sends nothing. No setting of the window program makes the
matcher accept more.

VoiceBridge has two programs. They use the same libraries and the same
configuration file format.

- The window program `voicebridge-ui` is the main way to use VoiceBridge. It
  has a screen for each group of settings, and it shows each result. The
  installer installs it. The first part of this guide is about this program.
- The console program `voicebridge` gives the same functions as commands in a
  terminal. The installer does not install it. See "The console program".

## What the window program needs

- Windows on a 64-bit x86 computer. The installer file is for x64.
- EDCoPilot, installed.
- A microphone.
- Microsoft Edge WebView2. Windows 11 has it. If WebView2 is not on the
  computer, the installer downloads it from Microsoft and installs it.
- For the local engine: approximately 1,160 MB of free disk space for the
  model download. After the download, the model uses approximately 671 MB.
- For a cloud service: an API key.

## Install the window program

The installer file is `EDCoPilot VoiceBridge_<version>_x64-setup.exe`. Download
it from the Releases page of the repository of this project. A developer can
also build the file (see "Build the installer").

The installer installs the program for the current Windows user only. It does
not need administrator rights.

The program folder is `%LOCALAPPDATA%\EDCoPilot VoiceBridge`. The installer is
in English.

The installer and the program have no code signature. Thus Windows SmartScreen
shows a warning before the first start of the installer.

To install the program, do these steps:

1. Start the installer file.
2. If SmartScreen shows "Windows protected your PC", make sure that the file
   comes from the Releases page of the repository of this project.
3. If the file comes from there, click "More info", then click "Run anyway".
4. Do the steps of the installer.
5. Start EDCoPilot VoiceBridge. The program file is `voicebridge-ui.exe` in the
   program folder.

The speech model is not in the installer. The Setup screen downloads it. See
"The Setup screen".

The program has no updater, and it does not start with Windows.

### Build the installer

This part is for developers. [BUILDING.md](BUILDING.md) tells you how to
install the tools, build the installer, and start a development build.

A development build never uses `%APPDATA%\VoiceBridge`. Before the start, set
the environment variable `VOICEBRIDGE_DEV_DATA_DIR` to an absolute scratch
folder.

The development build and the installed program have the same
identifier. Thus they cannot run at the same time.

## First start

At the first start, the program makes its data folder `%APPDATA%\VoiceBridge`.
The data folder holds these files:

| File or folder | Content |
| --- | --- |
| `voicebridge.toml` | The configuration. The program makes it from the example configuration, with absolute paths. |
| `synonyms.toml`, `sound_alikes.toml`, `fillers.toml`, `hotwords.txt` | Your copies of the word tables. |
| `aliases\` | Your copies of the alias files. |
| `models\` | The speech model. The model download makes this folder. |
| `logs\` | The utterance log and the audio files. The switches of the Diagnostics screen make this folder. |

The program ships no command map. It reads the commands from the EDCoPilot
profile on your computer. See "Where the commands come from".

The first configuration has these values:

- The EDCoPilot folder is `C:\EDCoPilot` when that folder exists. Else the
  value is empty.
- The model folder is empty.
- The local engine uses 4 threads, or fewer when the computer has fewer
  logical processors.
- The mode is push-to-talk with the key F13.
- The wake words are "copilot" and "co pilot".
- The hotwords, the utterance log, and the audio capture are off.

The program never overwrites a file in the data folder. If you delete one of
the copies, the next start copies the shipped file again.

At each start, the program runs the check of the configuration. If the check
finds a problem, the window opens the Setup screen. At the first start, the
empty model folder is such a problem.

The program also uses these folders:

- `%LOCALAPPDATA%\VoiceBridge\` holds the lock file of the voice session and
  the report of the last session start (`profile-report.json`).
- `%LOCALAPPDATA%\local.voicebridge.ui` holds the data of WebView2.

## Where the commands come from

EDCoPilot keeps its voice commands in its VoiceAttack profile
`EDCoPilot-Profile.vap` in the EDCoPilot folder. The program reads this file
each time that it loads the commands: at the start of a session, for `check`,
and for the Commands screen. The program never writes the file. It keeps no
copy of the commands.

The report of the build has these lines. `check` prints them, and the
Diagnostics screen shows them:

- `commands from the EDCoPilot profile: <n> commands, <m> left out`. The first
  number is the count of commands that the program can send. The second
  number is the count of commands that the program left out.
- `left out, <n> commands: <reason>, for example "<phrase>"`. One line for
  each reason of the commands that the program does not fully understand.
- `the profile has a newer format than the tested format, tell the developer
  when a command does not work`. This line shows only when the profile has
  another format than the tested format.
- `the EDCoPilot profile changed: <a> commands added, <r> removed, <l> now
  left out, <m> now in the map`. This line compares the profile with the
  profile of the last session start.
- `this program version reads the EDCoPilot profile in another way: ...`. The
  profile is the same, but a new version of this program gives other commands.

### What "left out" means

A command is left out when the program does not fully understand what the
command does in EDCoPilot. Many commands of the profile send nothing that the
program can write, for example commands that only change a setting. A left-out
command gives no match. The program never sends a wrong command in its place:
a miss is better than a wrong match.

When you say the phrase of a left-out command, the program sends nothing. It
also does not send a similar command. A word that sounds the same, a synonym,
a dropped filler word or a small difference in a word does not change the
phrase into a different command.

This applies to each left-out command that VoiceAttack recognises by voice. It
does not apply to a command that you switch off, a command without a voice
trigger, or a command that other commands run (a phrase in `((` and `))`).

`voicebridge check --left-out` prints one line for each left-out command, with
its reason. The Diagnostics screen has the same list under "Every left-out
command, with its reason".

### After an update of EDCoPilot

An update of EDCoPilot can change its profile. A running session keeps its
commands. The Status screen then shows the warning "EDCoPilot changed its
profile, stop and start the session to use the new commands". Stop the
session and start it again. The next start reads the new profile, and the
report shows what changed.

### Problems of the profile

If the program cannot use the profile, it starts no session. The check shows
one of these problems:

- `cannot read the EDCoPilot profile <path>: the file does not exist, set the
  EDCoPilot folder, or start EDCoPilot one time to make the file`. The path
  comes from the EDCoPilot folder. Set the correct folder. If the folder is
  correct, start EDCoPilot one time: it makes the file.
- `cannot read the EDCoPilot profile <path>: the file does not exist, set
  edcopilot.profile to the profile file`. The path comes from the key
  `edcopilot.profile`. Set the key to the profile file, or remove the key.
- `cannot read the EDCoPilot profile <path>: <cause>, set the EDCoPilot
  folder` or `..., set edcopilot.profile to the profile file`. The program
  cannot open the file for a different cause, for example Windows denies
  the access.
- `cannot read the EDCoPilot profile <path>: <cause>, another program holds
  the file now, try again`. Another program holds the file, also after a
  second read. Wait until the other program (for example EDCoPilot or
  VoiceAttack) closes the file, then check again.
- `the file <path> is not an EDCoPilot profile that the program understands:
  <cause>, <action>`. The file has damage, has more than 64 MiB, or has a
  format that the program does not know. The text ends with the action:
  - `update EDCoPilot`: the profile has an older format.
  - `tell the developer`: the profile has a format that the program does not
    know.
  - `use the profile file of EDCoPilot`: the file is not a VoiceAttack
    profile.
  - `try again, or start EDCoPilot one time to make the file again`: the file
    is empty or cut off.

The program uses no older command list in place of the profile.

### Configurations of an older version

An older version read the commands from a command map file. Its
configuration can have the keys `data.command_map` and `data.profile_vars`.
The program does not use these keys any more. A configuration with them
loads, and the program reads the EDCoPilot profile as usual. `check`, the
start of a session, and the window show one warning:

`data.command_map and data.profile_vars are not used any more, the program
reads the EDCoPilot profile`

To remove the warning, remove the two lines from the configuration. The
window program removes them at its start when they have the form that an
older version wrote (see "Update the window program").

A start with `--config` and the console program run no update step. Then
the warning stays until you remove the two lines.

## The window

The bar at the top shows the state of the session as a word: "Reading
status", "Starting", "Starting, dry run", "Running", "Stopping", "Stopped", or
"Stopped by error".

During a dry run, the bar also shows "Dry run: nothing is
sent". After a stop by an error, the bar shows the reason, for example
"Microphone error: <message>".

The navigation has six screens: Status, Setup, Microphone, Commands, Words,
and Diagnostics. A screen with changes that are not saved has the mark
"unsaved".

The Setup screen and the Microphone screen share one draft. "Save" on one of
the two screens writes the changes of both screens.

## The Setup screen

The top of the screen shows the result of the check:

- "Start is off until you fix these problems", with a list of the problems.
- "The check found no problem."

"Check again" runs the check again. Under "Warnings", the screen shows the
warnings of the check. Warnings do not stop the start.

### EDCoPilot folder

You must set "EDCoPilot folder". Type the folder, or click "Browse" and
select it. The program reads the voice commands and the name lists in this folder.
It writes only the request file there.

Below the field, the screen shows the profile file that the program reads,
for example "The program reads the commands from this profile:
C:\EDCoPilot\EDCoPilot-Profile.vap". If the folder is empty or the profile
does not load, the screen shows the problem text below the field. See
"Problems of the profile".

### Engine

"Engine" has two options:

- "Local (Parakeet)": the speech model runs on this computer. This is the
  primary engine.
- "Cloud (OpenAI-compatible)": a cloud service changes your speech to text.
  The service gets your voice over the network, and it needs an API key.

The settings of the other engine stay in the file.

### Model folder and the model download

The local engine needs "Model folder". Type the folder, click "Browse", or
click "Download the model".

Beside the button, the screen shows the size: "487 MB download, 671 MB on the
disk".

The program downloads the model
`sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8` from the release of the
sherpa-onnx project on GitHub. The license of the model is CC-BY-4.0.

The program contains the address and the SHA-256 hash of the archive. It uses
HTTPS only.

The download works in these steps:

1. The program makes sure that the disk has sufficient free space.
2. It downloads the archive into `%APPDATA%\VoiceBridge\models`. A progress
   bar shows the progress.
3. It checks the SHA-256 hash of the archive. If the hash is wrong, it deletes
   the archive and shows "the downloaded file is damaged".
4. It unpacks the model and checks the model files.
5. It writes the new model folder into the configuration file. The screen shows
   "The model is in <folder>."

"Cancel" stops the download. The screen then shows "The download stopped. A
new download continues from the same point." After a network error, a new
download also continues from that point when the server supports it.

The download continues when you open another screen. You can download the
model during a session. The session keeps the model of its start.

### Threads

"Threads" is the number of processor threads for the local model, from 1 to
the number of logical processors. The game needs the other cores. More than 4
threads did not make the model faster in the measurements.

### Hotwords

"Hotwords" switches on a second pass for names that the model often hears
wrong. The screen shows the costs: "about 700 MB more memory, about 1.2 s
longer start, and a second pass of about 0.1 s for each utterance that gives no
command."

When you switch the hotwords on, the program writes the path of
`%APPDATA%\VoiceBridge\hotwords.txt`. Edit the list on the Words screen. See
"Hotwords" in the configuration reference for the rules of the second pass.

### Cloud engine settings

For the cloud engine, the screen has these fields:

| Field | Meaning |
| --- | --- |
| "Server address" | The address of the service. |
| "Model" | The model name. |
| "Language" | Empty means the service detects the language. |
| "Time limit (seconds)" | The time limit for one request, from 1 to 600. |

### API key

The program keeps the API key in the Windows Credential Manager. The entry
has the service name "VoiceBridge". The key never goes into the configuration
file, and the window never shows the key.

The key belongs to the server of the saved "Server address". If you change the
server, enter the key again for the new server.

The line under "API key" shows the state:

- "No key is saved"
- "A key is saved for <server>"
- "The key comes from the environment variable <name>"

To save the key, do these steps:

1. If "Server address" has a change, click "Save" first.
2. Type the key in the field "Key".
3. Click "Save key". The screen shows "The key is saved."

When the Credential Manager has a key, the field is "New key" and the button
is "Replace key".

To remove the key, do these steps:

1. Click "Remove key".
2. Read the question "Remove the saved key from this computer?".
3. Click "Remove". The screen shows "The key is removed."

The key field is off while the server address or the variable name has a
change that is not saved. The help text then says "Save the server address
first" or "Save the variable name first".

A running session keeps the key of its start. If a session with the cloud
engine runs or starts when you save or remove the key, the screen adds "The
running session keeps the old key until you stop and start it." Stop the
session and start it again to use the new key.

The server address in the file can hold a user name, a password, a query
(after "?"), or a fragment (after "#"). These parts can be secret. The window
then shows only the scheme, the host, the port, and the path. The note under
"Server address" says that the file holds a secret or a query. The key field
is off. Correct the address in the file, or type a new address in the field
and save it.

Under "Advanced", the field "Name of the environment variable, not the key"
holds the name of an environment variable, for example `OPENAI_API_KEY`. The
program reads the key from the Credential Manager first, then from this
variable. The console program reads only the variable.

### Matching

| Field | Meaning |
| --- | --- |
| "Threshold" | Minimum confidence, from 0.0 to 1.0. A higher value gives fewer wrong actions. |
| "One-word phrases must be exact" | A phrase of one word needs confidence 1.0. Keep it on. |

### Updates

| Field | Meaning |
| --- | --- |
| "Check for updates at the start" | On: the program checks for a newer version about 10 seconds after the start and then every 24 hours. Off: the program checks only when you click "Check for updates" on the Diagnostics screen. The key is `updates.check`. |

The help line of the field is "The program asks the GitHub page of the
project for the newest version. It sends no data about you." Save writes the
switch. A running session does not need a restart for it. See "Updates".

### Save the settings

"Save" writes the changes. "Discard changes" removes them. The line "The
settings go to <path>." shows the configuration file.

Before the program writes the file, it checks the result. If the result has a
new problem, the program does not write the file. The screen then shows "Not
saved. The file did not change. Fix these problems:" and the list.

The program keeps the comments and the order of the file.

Another program can change the file while the window is open, for example an
editor. A save writes only the settings that you changed. If another program
changed one of these settings after the window read the file, the program
does not write the file. The list then shows "the configuration file changed,
read it again: another program changed a setting that this save changes too".
To continue, do these steps:

1. Click "Read the file again and keep my changes".
2. Examine the values of the file.
3. Click "Save" again.

A change of other settings does not stop the save. The save keeps that change.

The paths of the word tables must have the correct file name end. A synonym,
sound-alike, or filler file must end with `.toml`. A hotword list must end
with `.txt`. The program does not compare the case. The alias folder must be
a folder. Otherwise the save shows a problem, for example `data.synonyms must
name a file that ends with .toml: <path>`.

To set the program up for the first time, do these steps:

1. On the Setup screen, click "Browse" beside "EDCoPilot folder".
2. Select the EDCoPilot folder.
3. Under "Engine", select "Local (Parakeet)".
4. Click "Download the model".
5. Wait until the screen shows "The model is in <folder>."
6. Click "Save".
7. Make sure that the screen shows "The check found no problem."

## The Microphone screen

### Device

"Device" has "Default device" and the names of the connected input devices.
"Refresh" reads the list again. A device of the file that is not connected
stays in the list with "(not connected)".

### Mode

"How the program listens" has two options:

- "Push-to-talk" records only while you hold the talk button. This is the
  default mode.
- "Always listening" records each phrase that the microphone hears.

### Talk button

For push-to-talk, "Talk button" shows the key or the joystick button. With no
value in the file, it shows "F13 (default)". Most keyboards have no F13 key.

To set the talk button, do these steps:

1. Stop the session. While a session runs, "Press to set" is off.
2. Click "Press to set".
3. Press and release one key or one joystick button.
4. Read the line "The talk button is now <name>. Save to keep it."
5. Click "Save".

Only one input counts. A key combination is not a talk button. Each key
counts, also Tab.

The left, right, and middle mouse buttons cannot be the talk button. While
the capture runs, the other fields are off.

To stop the capture without a change, press Esc or click "Cancel". The screen
then shows "Cancelled. The talk button did not change." After 30 seconds
without a press, the capture stops.

A key removes the joystick button from the file. A joystick button removes
the key. These are the rules of `voicebridge bind`.

A key that types a
character gets that character, for example `<` on a German keyboard. See
"Write the key".

"Pre-roll (ms)" is the audio from before the press. "Release tail (ms)" is the
audio after the release. Each value is from 0 to 2000, and 0 is off. See
"Push-to-talk and always-listening".

### Wake words

"Wake words" is the list of wake words for always-listening. Push-to-talk
does not use wake words.

A wake word is optional, but we recommend it. In always-listening mode, the
microphone hears all speech near it. Without a wake word, a phrase that you
say to another person, or speech from a speaker near the microphone, can
start a command.

Type a word and click "Add". Click "Remove" to remove a word. When the list
has wake words, a phrase must start with a wake word, for example "copilot,
help voice log". The list has at most 64 wake words. Each wake word needs a
letter or a digit.

**Warning:** Always-listening without a wake word sends each match. The
program accepts an empty list, but it shows a warning. The Microphone screen
shows the warning before the save. The Status screen shows it while the
saved file listens always without a wake word:

> Always-listening without a wake word sends each match: each phrase that the
> microphone hears and that matches a command fires that command, also a
> phrase that you say to another person. A wrong match is worse than a miss.
> Add a wake word, or use push-to-talk.

"Voice threshold (dBFS)" is the level at which the program hears speech. A
higher value, for example -30, ignores more background noise. The screen
accepts values from -100 to 0.

## The Status screen

The Status screen starts and stops the session. It has three buttons:

- "Start" starts the session. Each match goes to EDCoPilot.
- "Start as dry run" starts the session, but the program sends nothing.
- "Stop" stops the session. An utterance in work can finish. After the stop
  request, no command goes to EDCoPilot.

The start buttons are off while the check has a problem. The screen then shows
"Fix the setup first: <N> problems." and the link "Open Setup".

During the start, the screen shows "Loading the speech model. This can take some
seconds." If the start fails, the screen shows "The session did not start"
and each problem.

The talk lamp shows "Talk button: no session", "Talk button up", or "Talk
button down: listening".

When a newer version is available, a note shows at the top of the screen:
"Version <new> is available. You have version <current>." The note has the
buttons "Install update" and "Later". See "Updates".

To test the program without a command to EDCoPilot, do these steps:

1. On the Status screen, click "Start as dry run".
2. Wait until the bar shows "Dry run: nothing is sent".
3. Hold the talk button and speak a command.
4. Read the result row.
5. Click "Stop".

### Result rows

Each utterance gives one row. The newest row is at the top. The list keeps
the newest 200 rows.

A row shows the result, the heard text, the confidence
in percent, the token, the marks, the reason, and the time. A row without
text shows "nothing heard".

| Result | Meaning |
| --- | --- |
| "Sent" | The program wrote the token to the request file, and EDCoPilot read it. |
| "Dry run" | The text matched. The program sent nothing. |
| "No match" | No command matched the text, or the text was empty. |
| "No wake word" | In always-listening mode, the text did not start with a wake word. |
| "Rejected" | A command matched, but the program did not use it. The row gives the reason, for example "a phrase of one word needs an exact match". |
| "Not read" | EDCoPilot did not read the token in 2 seconds. The reason is "withdrawn" or "could not be withdrawn". |
| "Engine error" | The speech engine failed for this utterance. The session continues. |
| "Send error" | The program could not write the request file. |
| "Dropped" | The program did not use the utterance. The reason is "waited <N> ms" or "the session stops". |

The marks are "sound-alike", "synonym", "filler", "hotwords", "glitch",
"clipping", and "very quiet". "Result lines" and "Console lines" in the
console part tell what each mark means.

The side panel "This session" or "Last session" shows the start lines of the
session. These are the start lines of `voicebridge run`. The panel also shows
"Vocabulary reloaded: <N> files, <M> names" after a change of the name lists.
It also shows "Audio glitches in the last session: <N>". "Warnings" shows the warnings
of the session.

### Changes during a session

The session reads the settings and the word tables when it starts. A save
during a session does not change the running session. The screens that save
show a note:

- Setup and Microphone: "The running session keeps its old settings until you
  stop and start it."
- Words: "The running session keeps the tables of its start. Stop the session
  and start it again to use the saved tables."
- Diagnostics: "Restart the session to use the change."
- Commands: "The test uses the saved settings. A running session keeps the
  settings that it started with: restart the session to use the saved
  settings."

To use the saved settings, click "Stop" on the Status screen, then click
"Start" again.

## The Commands screen

### Try a phrase

"Try a phrase" tests a typed phrase. Type the phrase and click "Test". The
test is always a dry run. It sends nothing to EDCoPilot, also while a session
runs.

The test treats the phrase as heard speech, with the saved settings. In
always-listening mode, the phrase must start with a wake word. Else the
result is "No wake word", and the screen names the wake words. The filler
words are also off in always-listening mode.

The result shows the result word, the confidence, the token, the reason, the
marks, and a sentence such as "The program would send this command. The test
sends nothing." The last line is the result line of the console.

The test does not use the speech engine. Thus it does not show what the engine
hears, and the hotwords do not apply. A phrase that works in the test can fail
live when the engine hears other words.

The test uses the saved files. A running session uses the files of its start.
Thus after a save, the test and the session can give different results until
you restart the session.

### The command list

The list shows the commands that work with your EDCoPilot files, in groups by
category. Each row shows the phrase and the token. An alias phrase has the
mark "alias".

The phrase uses the VoiceAttack syntax: `[a; b]` means "a" or
"b", and `[a;]` means that "a" is optional.

Type words in the filter field to narrow the list. A row stays in the list
when the phrase, the token, or the category has each word.

"Show commands that do not work" adds the commands that cannot run. Each of
these rows gives the reason.

The screen shows 200 rows at a time. Click "Show 200 more" for the next rows,
or type a filter.

After the commands, the screen shows the composite groups,
for example the commodity searches. The last line is the summary of the list.

## The Words screen

The Words screen edits your copies of the word tables. It has one tab for
each table: "Aliases", "Synonyms", "Sound-alikes", "Filler words", and
"Hotwords". The line "File: <path>" shows the file of the tab.

Each tab has the rows, "Add", "Remove", the result of the check, "Save", and
"Discard changes and read the file again".

### The check

While you type, the screen checks the rows as `voicebridge check` does. It
uses the real command data.

- A problem switches "Save" off. The screen shows "A problem stops the save.
  Change the rows."
- A warning needs a confirmation. Click "I read the warnings: save is
  permitted", then click "Save".
- "No problem and no warning." means that the full check found nothing.

After a save, the screen shows "Saved. The running session keeps the old table
until you stop and start it again." when a session runs.

The check needs the EDCoPilot folder. Without it, each check gives the problem
"the check is not complete: edcopilot.dir is empty, set the EDCoPilot folder",
and the screen refuses each save.

### The tables

**Aliases.** An alias is an extra spoken phrase for one command, for example
in your language. The first column is the token template of the command,
exactly as the command list shows it. The second column has the phrases, one
on each line. Example: `OpenVoiceLogPanel` with the phrase
`sprachprotokoll [öffnen; zeigen;]`. To find the token of a command, search
the command list of the Commands screen, or run `voicebridge commands --all`
(see "`commands`").

Select the alias file with "Language". "New language" makes a new file, for
example `fr`, at the first save.

Danger: a
short or broad phrase can match speech that is for another command. The screen
does not warn about a broad phrase. Keep each phrase specific.

**Synonyms.** A synonym is your own words for a phrase of EDCoPilot. It works
in every command with that phrase. Example: "navigate to" for "set course to"
and "set course for". Then "navigate to Sol" works as "set course to Sol".

Danger: a key with the words of a command phrase can change that command. The
check gives a warning for such a key. For example, the key "head for" gives a
warning when a command phrase starts with "head for".

**Sound-alikes.** A sound-alike is a real word that the engine writes for a
command word. Example: "by" for "buy". Each row has exactly one meant word.

Danger: a heard word that is itself a command word in the same place can give
a different command. For example, the shipped file has no row "sale" for
"sell", because a command phrase has the word "sale" in the same place. The
check cannot find this danger. Test each new row on the Commands screen.

**Filler words.** A filler word is a small word that the program can drop when
nothing else matches. Example: "uh" with "Can be the last word (may_be_last)",
and "the" with "Never the last word (not_last)".

Danger: a word that changes
the meaning can give a wrong command. Do not add "to", "of", "in", "not",
"no", "on", "off", or "dont". The check gives a warning for a filler word that
is a word of a name, for example "the" in a station name that starts with
"The". The filler words
apply only in push-to-talk mode.

**Hotwords.** A hotword is a name that the local engine pulls toward in a
second pass. Example: `Sol`.

Danger: a hotword near another hotword, a name,
or a command word can change one name into another name. The check gives a
problem for such a hotword, and the save is not possible. For example, the
screen does not save a hotword beside a name of the name lists that differs
in one letter.

When the hotwords are off, the tab says "The hotwords are off." and has the link "Open Setup". The cloud engine
ignores the hotwords.

### Files that the screen does not edit

The screen writes a table only to a file with the correct end: `.toml`, or
`.txt` for the hotwords. A configuration that you edit by hand can name
another file. The screen then shows the rows, but the save shows "the Words
screen writes this table only to a file that ends with .toml, correct the
path in the configuration file: <path>".

The screen refuses a file with content that the rows cannot hold. The read
and the check then show a text that ends with "edit the file by hand".
Examples:

- An unknown key or table.
- A value that is not a text or a list of texts.
- A change of a list that has a comment inside.
- A file that is larger than 256 KiB.

Open such a file in a text editor. The screen also has these limits: 1000
rows, 300 characters in one entry, and 64 values in one row.

If another program changes the file after the screen read it, the screen
refuses the save. It then shows "Another program changed the file after this
screen read it." Click "Read the file again and keep my rows", check the
result, and save again.

To add a synonym, do these steps:

1. On the Words screen, click "Synonyms".
2. Click "Add".
3. In "Your words", type your phrase, for example "head for".
4. In "EDCoPilot phrases (one on each line)", type the EDCoPilot phrase, for
   example "set course for".
5. Read the result of the check.
6. If the check shows a warning, click "I read the warnings: save is
   permitted".
7. Click "Save".
8. On the Commands screen, test a phrase with your words, for example "head
   for sol".
9. If a session runs, stop the session and start it again.

## The Diagnostics screen

The Diagnostics screen has data for tuning. A change applies at the next start
of the session.

| Switch | What it does |
| --- | --- |
| "Utterance log" | One line for each utterance: the time, the result, the heard text, the token, the confidence, the times, and the levels. On writes `logs\utterances.tsv` in the data folder. |
| "Audio capture" | The audio of each utterance as a WAV file. On writes `logs\audio` in the data folder. The program keeps the newest 500 files. |

The two switches are off by default. A switch saves at once. It does not use
the draft of the Setup screen. "Off" removes the path from the configuration.
"On" again uses the default folder.

The audio files hold your voice. They stay on this computer. The program does
not send them to a service. Only the cloud engine gets your voice, in its
normal speech-to-text request.

Delete the files when you do not need them. "Collect data for tuning" in the console part has the format of the log.

"Open folder" opens the folder of the log, of the audio files, or the data
folder. Under "Data folder", the screen shows the path of the data folder and
the text "An uninstall keeps this folder. Delete it by hand to remove your
settings, the model and the recordings."

"Commands from the EDCoPilot profile" shows the report of the build: the
lines of `check` that "Where the commands come from" describes. If the profile
does not load, the panel shows the problem text in place of the report. Open
"Every left-out command, with its reason" to see each left-out command. The
list shows 200 lines at a time. "Show more" shows the next lines. Type words
in the filter to find a command.

"Output of the check" shows the lines of `voicebridge check` for the
configuration file. "Check again" runs the check again.

"Updates" shows "You have version <current>." and the result of the last
update check:

| Line | Meaning |
| --- | --- |
| "You have the latest version. Last check: <time>." | The last check found no newer version. |
| "Version <new> is available." | A newer version is available. The Status screen has the note. |
| "The last check failed: <cause>." | The program could not get the update information. |
| "The program does not check for updates (updates.check is false)." | The switch is off, and you did not click "Check for updates" in this run. |
| "The program did not check for updates yet." | The first automatic check comes about 10 seconds after the start. |

"Check for updates" asks for the newest version now. It also works when the
switch is off. A developer build shows "This is a developer build. It has no
updates."

## The notification area

The close button of the window does not stop the program. It hides the
window, and the session continues. The first time in each run, the program
shows this message:

> VoiceBridge keeps running in the notification area. A running session
> continues.
>
> To show the window again, click the VoiceBridge icon in the notification
> area. To end the program, right-click the icon and select Quit.

A left click on the icon shows the window. The tooltip of the icon shows the
state, for example "VoiceBridge: session running" or "VoiceBridge: dry run,
nothing is sent".

The menu of the icon has these entries:

| Entry | What it does |
| --- | --- |
| "Show VoiceBridge" | Shows the window. |
| "Start..." | Shows the window with the Status screen. When the check has a problem, it shows the Setup screen. It does not start a session. |
| "Stop" | Stops the session. It is on only while a session runs or starts. |
| "Quit" | Stops the session, then ends the program. |

When you log off or shut down Windows, the program stops the session before it
ends. Windows can end the program when the stop takes too long.

## One instance

The window program runs only one time. A second start
shows the window of the first program, and then the second program ends. The
second program ignores its arguments.

Known limits:

- If the first program runs with administrator rights and the second program
  does not, the second program ends without a window.
- A second start while the first program ends can also end without a window.
  Start the program again.

The console program and the window program can run at the same time. But
only one voice session can use a request file.

A second session gets the
problem "a voice session for <request file> is already running". This is also
true for a dry run.

## Updates

The window program can update itself. The program asks the GitHub releases
page of the project for the file `latest.json`. That file names the newest
published version. The program sends no data about you. A draft or a
pre-release on GitHub does not count.

The program checks at these times:

- About 10 seconds after the start, when `updates.check` is true. The check
  does not stop the start.
- Every 24 hours while the program runs, when `updates.check` is true.
- When you click "Check for updates" on the Diagnostics screen. This also
  works when `updates.check` is false.

A development build never checks.

Only a newer version counts. An equal or older version is "the latest
version". The program never installs an update without your click.

A check that the program starts by itself and that fails shows nothing on
the Status screen. The Diagnostics screen shows "The last check failed:
<cause>." A check that fails after a click on "Check for updates" shows "The
program could not check for updates: <cause>. Try again later."

### Install an update

1. On the Status screen, find the note "Version <new> is available. You have
   version <current>."
2. Optional: open "What is new" to read the changes. The note also shows the
   address of the release page as text. Copy it into your browser to see the
   page.
3. If a session runs, click "Stop". While a session runs, starts, or stops,
   "Install update" is off and the note shows "Stop the session first."
4. Click "Install update".
5. Wait. The note shows "Downloading the update: <n> of <m> MB." Click
   "Cancel" to stop the download. The program does not change.
6. After the download, the program checks the signature of the installer.
   The note then shows "The update is ready. The program closes and the
   installer starts."
7. The program closes. The installer shows a small progress window and asks
   no questions.
8. The installer starts the new version of the program.

During the download and the install, you cannot start a session. A start
then gives "The update is in progress."

"Later" hides the note until the next start of the program or the next
click on "Check for updates".

Your configuration, your word tables, the model, and the API key stay as they
are. The installer replaces only the program folder.

### When the update fails

If the download or the signature check fails, the note shows "The update did
not install: <cause>. The program is not changed." The program never starts
an installer with a wrong signature. Click "Install update" again later.

If the update fails again, install the new version by hand:

1. Open the Releases page of the project on GitHub.
2. Download `EDCoPilot-VoiceBridge_<version>_x64-setup.exe` of the newest
   release.
3. Stop the session and quit the program (right-click the icon in the
   notification area, then select Quit).
4. Start the installer.

## Update the window program

To update the program by hand, run the installer of the new version. The data
folder stays as it is.

- The program reads the commands from the EDCoPilot profile, as before.
- Your copies of the word tables stay as they are. The update does not add
  the new entries of the shipped tables to your copies.

To get the new entries of a shipped table, do one of these steps:

- Compare your copy with the shipped file and add the new entries. Find the
  shipped files in `%LOCALAPPDATA%\EDCoPilot VoiceBridge\resources\data`, and
  the shipped alias files in `resources\aliases`.
- Delete your copy, then start the program again. The program copies the new
  shipped file. Your own entries in that copy are then lost.

An older version wrote `data.command_map` and `data.profile_vars` into your
configuration, with the paths of its map files in the program folder. The
program does not use these keys any more. At the first start after the
update, the program removes each key that is empty or has the path that the
older version wrote. The comment line of the older example above the keys
goes too. The other settings, the other comments, and the line ends of the
file stay.

The Status screen and the
Diagnostics screen then show "The program reads the commands from the
EDCoPilot profile now. The update removed data.command_map and
data.profile_vars from the configuration." When the update removed only
one key, the note names that key. The next start does not show this note.

A key with another value (for example a map file of your own) stays in the
file. The program does not use it. Each start then shows a note such as
"data.command_map is not used any more. The program reads the commands from
the EDCoPilot profile. You can remove the line from the configuration."

If the program cannot write the configuration (for example, the file is
read-only), the note is "The update could not remove data.command_map and
data.profile_vars from <file>: <cause>. Make the file writable, or remove the
two lines by hand." You can still start a session.

The update does not delete old map files. Old files
`resources\data\command_map_v3.json` and `profile_vars.json` can stay in the
program folder. The program does not read them.

## Uninstall the window program

Uninstall the program in the settings of Windows. The uninstaller has the option "Delete
the application data". This option deletes only the data of WebView2 in
`%APPDATA%\local.voicebridge.ui` and `%LOCALAPPDATA%\local.voicebridge.ui`.

The data folder `%APPDATA%\VoiceBridge` always stays. It holds the
configuration, the word tables, the model, and the recordings. To remove them,
delete the folder by hand.

The folder `%LOCALAPPDATA%\VoiceBridge` also stays. It holds the lock file of
the voice session and the report of the last session start
(`profile-report.json`).

The report lists the command phrases of your EDCoPilot profile. To remove
it, delete the folder by hand.

To remove the API key, click "Remove key" on the
Setup screen before you uninstall the program.

## Troubleshooting

| Text or symptom | Cause | What to do |
| --- | --- | --- |
| The window does not show. | The window is in the notification area. | Click the VoiceBridge icon in the notification area. |
| "edcopilot.dir is empty, set the EDCoPilot folder" | The configuration has no EDCoPilot folder. | Set "EDCoPilot folder" on the Setup screen. |
| "stt.parakeet.model_dir is empty, the parakeet engine needs it" | The local engine has no model folder. | Click "Download the model", or select the model folder. |
| "stt.parakeet.model_dir is not a directory: <folder>" | The model folder moved, or it does not exist. | Download the model again, or select the correct folder. |
| "voice commands file not found: <path>" | The EDCoPilot folder is wrong. | Select the correct EDCoPilot folder. |
| "environment variable <name> is absent or empty, it must hold the API key" | The cloud engine has no API key. | Save the key under "API key" on the Setup screen. |
| "the saved API key belongs to <server>, enter the key again for <server>" | The server address changed after you saved the key. | Save the key again for the new server. |
| "a voice session for <request file> is already running" | Another session uses the request file, for example `voicebridge run`. | Stop the other session. |
| "input device "<name>" not found; input devices: <list>" | The microphone of the file is not connected. | Connect the microphone, or select another device on the Microphone screen. |
| "Microphone error: <message>" in the bar | The microphone stopped during the session, or the default microphone of Windows changed. | Connect the microphone, then click "Start". |
| "the downloaded file is damaged" | The hash of the archive is wrong. | Click "Download the model" again. If the text comes again, the release changed, and a program update is necessary. |
| "the model archive on the server changed, a program update is necessary" | The archive on the server has another size. | Select a model folder that you downloaded by hand, or wait for a program update. |
| "not enough free disk space in <folder>: the download needs <N> MB, the disk has <M> MB" | The disk is too full. | Make free space, then download again. |
| "a model download is already running" | Another download runs, also in another program. | Wait until it ends. |
| "stop the session first: the session reads the talk button, and two readers do not work" | "Press to set" during a session. | Stop the session, then set the talk button. |
| "a talk button capture is in progress, finish it first" | A start during "Press to set". | Press the talk button, or cancel the capture. |
| "Not set: no key or button was pressed in 30 seconds" | No input came during the capture. | Click "Press to set" again. |
| The joystick button does nothing while the game has the focus. | The joystick input can need a window with focus. This is not proven. | Use a keyboard key as the talk button. Bind the joystick button to that key in the software of the joystick. |
| Many rows show "Not read". | EDCoPilot is not in operation. | Start EDCoPilot. It needs 1 to 2 minutes to start. |
| Rows show "very quiet" or "clipping". | The microphone level is too low or too high. | Change the level of the microphone in Windows. |
| A word table shows a text that ends with "edit the file by hand". | The file has content that the screen cannot show. | Edit the file in a text editor. |
| "the file changed after the screen read it: read the table again, then make the change again" | Another program changed the table file. | Click "Read the file again and keep my rows", then save again. |
| A second start shows no window. | The first program runs with administrator rights, or it ends now. | Start both programs without administrator rights. |

## Where the files are

| Path | Content |
| --- | --- |
| `%LOCALAPPDATA%\EDCoPilot VoiceBridge` | The program, and the shipped data in `resources`. |
| `%APPDATA%\VoiceBridge` | Your configuration, word tables, model, log, and audio files. |
| `%LOCALAPPDATA%\VoiceBridge` | The lock file of the voice session, and the report of the last session start (`profile-report.json`). |
| `%APPDATA%\local.voicebridge.ui`, `%LOCALAPPDATA%\local.voicebridge.ui` | The data of WebView2. |
| Windows Credential Manager, service "VoiceBridge" | The API key. |

## The console program

The console program `voicebridge` has the same functions as the window
program, as commands in a terminal. The rest of this guide is about the
console program and the configuration file. The configuration reference and
the rules of the matcher apply to both programs.

The installer does not install the console program. To use it, build it from
the repository.

To use the configuration file of the window program, add
`--config "%APPDATA%\VoiceBridge\voicebridge.toml"` to each command. The
console program does not read the API key from the Credential Manager. For
the cloud engine, set the environment variable.

## What the console program needs

- Windows.
- The Rust toolchain, to build the program.
- EDCoPilot, installed.
- A microphone.
- For the local model: about 640 MiB of disk space.
- For the cloud service: an API key.

[BUILDING.md](BUILDING.md) tells you how to install the tools. To build the
program, run this command in the repository root:

```text
cargo build --release -p voicebridge
```

The program is then `target\release\voicebridge.exe`.

## Get the local model

The local engine uses the sherpa-onnx Parakeet TDT 0.6b v3 int8 model. The model
name is `sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8`. The model works with many
languages.

The Setup screen of the window program downloads the model. For the console
program, do these steps:

1. Download this archive (about 465 MiB):
   `https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2`
2. If the download gives HTTP 504, use this address with the header
   `Accept: application/octet-stream`:
   `https://api.github.com/repos/k2-fsa/sherpa-onnx/releases/assets/719769366`
3. Unpack the archive to a directory outside the repository.
4. Make sure the directory has these four files:
   `encoder.int8.onnx`, `decoder.int8.onnx`, `joiner.int8.onnx`, and `tokens.txt`.
5. Write the path of this directory in `stt.parakeet.model_dir`.

The model directory is not part of the repository.

## Make the configuration file

1. Copy `app/voicebridge.example.toml` to `voicebridge.toml` in the repository root.
2. Set `dir` in `[edcopilot]` to your EDCoPilot folder.
3. Set `model_dir` in `[stt.parakeet]` to your model directory.
4. Run `voicebridge check`.

The console program reads `voicebridge.toml` in the current directory. To use
another file, add `--config <FILE>` to the command. The window program uses
`%APPDATA%\VoiceBridge\voicebridge.toml`.

A relative path in the file starts at the directory of the configuration file.
The default `[data]` paths are relative. Thus the file belongs in the repository
root. If you put the file in another directory, change the `[data]` paths.

The program does not accept a key that it does not know. It reports the key
and the file name.

### `[edcopilot]`

| Key | Default | Meaning |
| --- | --- | --- |
| `dir` | none, required | The EDCoPilot folder. You can leave it empty only when you set the four paths below. |
| `request_file` | `<dir>\EDCoPilot.request.txt` | The file where the program writes the token. |
| `voice_commands_ini` | `<dir>\working\EDCoPilot.VoiceCommands.ini` | The grammar file of EDCoPilot. |
| `dynamic_vars_dir` | `<dir>\Dynamic var files` | The folder with the name lists. |
| `profile` | `<dir>\EDCoPilot-Profile.vap` | The VoiceAttack profile of EDCoPilot. The program reads the commands from it. See "Where the commands come from". |

If `dir` is empty and one of the first three paths is not set, `check` shows
the problem `edcopilot.dir is empty, set the EDCoPilot folder`. If only
`profile` is not set, the problem is `edcopilot.dir and edcopilot.profile are
empty, set the EDCoPilot folder or edcopilot.profile`. A path that
defaults from an empty `dir` stays empty. The program then never reads or
writes a file in the current directory.

The program makes the directory of the configuration file an absolute path
before it uses a relative path of the file. Thus `--config voicebridge.toml`
with a relative `dir` gives an absolute request file in that directory. The
lines of `check` show a relative log path or hotword path as the file has it.
If the request file is still not an absolute path, `check` and the start show
the problem `the request file is not an absolute path: <path>, give the
configuration file with its full path`.

### `[data]`

| Key | Default | Meaning |
| --- | --- | --- |
| `command_map` | none | Not used any more. The program reads the EDCoPilot profile. A file with the key loads and gives one warning. See "Configurations of an older version". |
| `profile_vars` | none | Not used any more, as `command_map`. |
| `aliases_dir` | `core/aliases` | The folder with the alias files. |
| `sound_alikes` | `core/data/sound_alikes.toml` | The sound-alike words. See "Sound-alike words". |
| `synonyms` | `core/data/synonyms.toml` | Your own words for a phrase. See "Use your own words for a phrase". |
| `fillers` | `core/data/fillers.toml` | Small words that the program can drop ("uh", "the"). See "Filler words". |

If the synonym file or the filler file does not exist or is not valid TOML,
`check` shows a problem. The program then works without these words.

An old configuration without the `fillers` key gets the default path. The default
path starts at the folder of the configuration file. If your configuration
file is not in the repository root, set `fillers` to the full path of
`core/data/fillers.toml`.

If the sound-alike file does not exist or is not valid TOML, `check` shows a
problem. The program then works without sound-alike words.

`check` also shows a problem for each entry that it cannot use. Examples are a
word that maps to itself, a key with a space or punctuation, an empty value,
and two keys that differ only in case.

### `[matching]`

| Key | Default | Meaning |
| --- | --- | --- |
| `threshold` | `0.8` | Minimum confidence, from 0.0 to 1.0. A higher value gives fewer wrong actions. |
| `exact_single_word` | `true` | A phrase of one word needs confidence 1.0. |

### `[microphone]`

| Key | Default | Meaning |
| --- | --- | --- |
| `device` | `""` | Part of the device name. An empty value means the default device. |
| `mode` | `"push_to_talk"` | `"push_to_talk"` or `"always_listening"`. |
| `vad_threshold_dbfs` | `-40.0` | Speech starts above this level in dBFS. |

To see the device names, run `voicebridge devices`. The program first looks for
an exact name, without regard to case. Then it uses the first name that
contains your text.

### `[push_to_talk]`

| Key | Default | Meaning |
| --- | --- | --- |
| `key` | `"F13"` | A keyboard key or a side mouse button. See "Write the key". |
| `joystick` | not set | A joystick button, for example `{ device = "MOZA", button = 5 }`. |
| `pre_roll_ms` | `250` | The milliseconds of audio before the press that the recording keeps. From 0 to 2000. 0 switches it off. |
| `release_tail_ms` | `300` | The milliseconds of audio after the release that the recording keeps. From 0 to 2000. 0 switches it off. |

Set `key` or `joystick`, not both. If you set neither, the program uses `F13`.
To set the key or the button without an editor, run `voicebridge bind`, or use
"Press to set" on the Microphone screen.

#### Write the key

`key` accepts four forms:

| Form | Examples | Meaning |
| --- | --- | --- |
| Name | `"F13"`, `"RControl"`, `"Space"`, `"A"`, `"Key1"`, `"Numpad0"` | A key name. The program ignores case. |
| One character | `"<"`, `"ö"`, `"#"`, `"+"` | The key that types this character on the active keyboard layout. |
| Virtual-key code | `"vk:226"`, `"vk:0xE2"` | A Windows virtual-key code from 1 to 254, decimal or hexadecimal. |
| Side mouse button | `"Mouse4"`, `"Mouse5"` | The two side buttons of the mouse. |

To find the text for a key, run `voicebridge inputs` and press the key.

A character must type without a modifier on the active keyboard layout. A
character that needs Shift, Ctrl, Alt, or AltGr is an error. On a German
keyboard, `"<"` is correct, but `">"` and `"@"` are errors.

For such a key, write
the character without the modifier, write the `vk:` code, or run
`voicebridge bind`.

If the keyboard layout cannot type the character, `check` reports a problem.

The program finds the key with the keyboard layout that is active when the
program starts. Use the same layout in the game.

The characters, the `vk:` codes, `Mouse4`, and `Mouse5` work only on Windows.

The names `Grave`, `Minus`, `Equal`, `LeftBracket`, `RightBracket`,
`BackSlash`, `Semicolon`, `Apostrophe`, `Comma`, `Dot`, and `Slash` name
the keys where a US keyboard has them. On other keyboards, write the character
of the key.

The program does not accept the left, right, and middle mouse buttons. The game
and the desktop need them.

`NumpadEnter` does not work on Windows. Windows gives the same code for the two
Enter keys. Use `Enter` or another key.

### `[always_listening]`

| Key | Default | Meaning |
| --- | --- | --- |
| `wake_words` | `["copilot", "co pilot"]` | Optional. The text must start with one of these words. An empty list means no wake word. We recommend a wake word (see "Push-to-talk and always-listening"). |

### `[stt]`

| Key | Default | Meaning |
| --- | --- | --- |
| `engine` | `"parakeet"` | `"parakeet"` for the local model, or `"openai_compatible"` for a cloud service. |

### `[stt.parakeet]`

| Key | Default | Meaning |
| --- | --- | --- |
| `model_dir` | `""` | The model directory. Required for the `parakeet` engine. |
| `threads` | 4, or the number of logical cores if fewer | The number of processor threads that the local model uses. |
| `hotwords_file` | `""` | The hotword list. An empty value switches the hotwords off. The shipped list is `core/data/hotwords.txt`. The window program uses the copy `hotwords.txt` in its data folder. |
| `hotwords_score` | `1.5` | The bias toward the hotwords, from 0.1 to 4.0. |

`threads` must be from 1 to the number of logical cores of the machine.

#### Hotwords

Hotwords are names that the model often hears wrong. For example, the model
can hear "Sol" as "toll". The hotword list makes these names more probable
in a second pass. The hotwords are off by default.

To switch the hotwords on, set `hotwords_file = "core/data/hotwords.txt"` in
`[stt.parakeet]`. Then run `voicebridge check`. In the window program, use
the switch "Hotwords" on the Setup screen.

The second pass may confirm a command. It may not change a command into a
different command. The program uses it only in these cases:

1. The engine makes a text with greedy search, as before.
2. If this text gives a command, the program uses it. The hotwords do not
   apply.
3. If the matcher refused the text on purpose (two commands are equally
   good), the result stays. There is no second pass. An empty text (silence)
   also gets no second pass.
4. If the text gives no match or a rejected match, the engine makes a second
   text.
5. The program uses the second text only when it gives a command and when it
   changes only hotwords. The two texts must have the same number of words.
   Each changed word must be part of a complete hotword phrase in the second
   text. The case and the punctuation do not count.
6. After a rejected match, the command of the second text must also have the
   token of the rejected match.
7. If the files in `Dynamic var files` change during the second pass, the
   program does not use the second text.
8. If the clip is older than 5 seconds after the second pass, the program
   does not send the command of the second text.

In all other cases the result of the first text stays. Thus each utterance
that gave a command without hotwords gives the same command with hotwords.
The result line and the log show the mark `hotwords` when the command came
from the second text.

The hotword file has one phrase on each line. A line that starts with `#` is a
comment. A phrase must not contain `/` or `:`.

The case is important. Write a name as the model writes it, for example `Sol`
and not `sol`. `check` shows a problem for a word that the model cannot spell.
`check` shows a warning when the file has no hotword.

A hotword must not be near another hotword or a name that the matcher can
send. "Near" means that the matcher accepts one word as an inexact form of
the other word. A hotword must also not be equal to or near a word of a
command phrase, for example "course" or "three". `check`, `run`, and `replay`
refuse such a list and name the two words.

For example, two system names that differ only in one letter are near. In a
test with two such names in the list, the second pass changed the first name
into the second name.

The program reads the hotword file only at the start. When the files in
`Dynamic var files` change, the program checks the hotwords again before the
next second pass. If a hotword is then near a name, the program prints a
warning and switches the second pass off until the next start.

Keep the list short. Use only names that the model gets wrong. A long list or
a high score makes the text worse. The names of the `Dynamic var files` do
not go into the list, because a longer list gave worse results.

The shipped list has 7 system names. In a measurement with real clips, the
second pass gave no clear gain. Thus the hotwords are off by default.

The second pass has costs:

- The engine loads the model a second time. This uses approximately 700 MB
  more memory and adds approximately 1.2 seconds at the start.
- An utterance that gives no command with the first text gets a second
  decode, approximately 0.1 seconds. Other utterances take no more time.
- The `transcribe` time of the result line includes the second pass.

Measured times of the model: on a machine with 32 logical cores, the model
loaded in about 1.0 to 1.6 seconds. A command of 2 to
2.5 seconds took about 0.06 seconds with 4 threads. With 1 thread it took about
0.13 seconds. With 8 threads it was not faster than with 4 threads. With 16
threads it was slower. In a live check on the test computer, the local engine
needed about 0.065 seconds for one spoken command.

### `[stt.openai_compatible]`

| Key | Default | Meaning |
| --- | --- | --- |
| `base_url` | `"https://api.openai.com/v1"` | The address of the service. |
| `api_key_env` | `"OPENAI_API_KEY"` | The name of the environment variable that holds the API key. Only letters, digits, and `_`. |
| `model` | `"gpt-4o-mini-transcribe"` | The model name. |
| `language` | `""` | An empty value means the service detects the language. |
| `timeout_seconds` | `10` | The time limit for one request. |

The API key is never in the configuration file. The file holds only the name of
the environment variable. For the console program, set the variable before you
start the program.

The window program reads the key from the Windows Credential Manager first,
then from the variable. See "API key" in "The Setup screen".

If `api_key_env` is not the name of a variable, `check` shows a problem. The
program refuses a value that looks like a key.

If `base_url` holds a user name or a password, `check` shows a problem for
both engines. The text shows only the scheme and the host, never the user
name or the password:

```text
problem: stt.openai_compatible.base_url (https://<host>) is not valid: it must not hold a user name or a password
```

If the cloud engine uses `http://` to a server on another computer, `check`
shows this line:

```text
warning: the cloud engine sends the API key without encryption
```

Use `https://`. The warning does not come for `localhost` or a loopback
address.

The window program adds a pointer to the key field to the problem of an
absent key: "…, or save a key in the field "API key" on the Setup screen".
The console text stays as it is.

### `[diagnostics]`

| Key | Default | Meaning |
| --- | --- | --- |
| `log_file` | `""` | A file that gets one line for each utterance. An empty value means no log. |
| `save_audio_dir` | `""` | A directory for the audio of each utterance. An empty value means no audio. |

Use a directory for `save_audio_dir` that holds nothing else. The program
deletes its oldest audio files in that directory. See "Collect data for
tuning".

### `[updates]`

| Key | Default | Meaning |
| --- | --- | --- |
| `check` | `true` | The window program checks for a newer version at its start and every 24 hours. `false` switches the automatic check off. "Check for updates" on the Diagnostics screen still works. |

The console program does not check for updates. The address of the update
server is not a setting. The table has no other key.

## The commands

All commands accept `--config <FILE>`.

### `check`

`check` finds problems in the configuration and in the data files. It does not
start the microphone. It does not load the speech model.

```text
voicebridge check
```

It prints one line with the load report. The line shows the number of commands
that it loaded, how many of them can run, the number of composite parts, and
how many have grammar that is not resolved. It also shows the vocabulary files,
the names, the alias phrases, the sound-alike entries, the synonym entries, and
the filler words. In `always_listening` mode, the line adds `(off in
always_listening mode)` after the filler words. A composite part is a prefix
or a suffix (see "Commodity searches").

`check` prints a `warning:` line for a synonym key that is also in a command
phrase. It also prints a `warning:` line for a filler word that is a word of a
name, for example "the" in a station name that starts with "The". The
program does not drop the word
where the full name is in the text.

`check` also prints a `warning:` line for an alias key that no command takes:
`alias file <path>: the token "<key>" matches no command that works, the
program ignores its phrases`.

For a token with `{CMD}`, the line is `alias file <path>: the token "<key>"
needs the English command text, an alias cannot give it, the program sends no
command for its phrases`. The program keeps these phrases. When you say one of
them, the program sends no command, also when a different command is near.

`check` also prints a `warning:` line for each command that has a phrase of a
command that the program cannot send: `the phrase "<text>" of the command
"<phrase>" is also a phrase of a command that the program cannot send, the
program does not send a command for it`. The other command needs a value, or
the program left it out.

When you say that text, the program sends no command. The other phrases of
the command work.

`check` prints at most 10 of these lines,
then `... and <n> more`. The Commands screen of the window shows the same
lines.

If you set `log_file` or `save_audio_dir`, `check` prints the path in a line
`utterance log: <path>` or `audio capture: <dir>`. If the directory of
`log_file` does not exist, `check` shows a problem. If `log_file` is a
directory, `check` also shows a problem.

In `push_to_talk` mode with a valid key or joystick, it prints the talk button.
For a key, the line shows the key as you wrote it and the virtual-key code that
the program reads. The code is not shown on other systems than Windows.

```text
push-to-talk: key "<" (vk:226)
```

Then it prints one `problem:` line for each problem. If there is no problem, it
prints `configuration is valid`. Otherwise it prints the number of problems.
The exit code is 0 if the configuration is valid. Otherwise it is 1.

Use `check` first. A wrong path to the voice commands file removes many commands.
`check` shows this as a problem.

### `text`

`text` matches a phrase that you type. It does not use the microphone or a
speech engine. It does not use wake words.

```text
voicebridge text "help voice log"
```

Without `--send`, this is a dry run. The program shows the match but writes
nothing. It prints one result line (see "Console lines").

```text
voicebridge text "help voice log" --send
```

With `--send`, the program writes the token to the request file. Start EDCoPilot
before you use `--send`.

`text` prints a `warning:` line for each missing path. It exits with code 0
after it prints a result line, also when there is no match. If
`matching.threshold` is not from 0.0 to 1.0, `text` prints a `problem:` line
and exits with code 1.

### `devices`

`devices` prints the names of the input devices, one name for each line.

```text
voicebridge devices
```

If it finds no device, it prints `no input device found`. The exit code is 0.

### `inputs`

`inputs` shows the keys and joystick buttons that you press. Use it to find the
text for a key or the number of a joystick button.

```text
voicebridge inputs
```

It prints `press keys or joystick buttons, Ctrl+C stops`. Then it prints a line
each time the pressed inputs change. A line with nothing pressed is
`(nothing pressed)`. Press `Ctrl+C` to stop. The exit code is 0.

A key has the form `key <text>`. You can put the text in `key` in
`[push_to_talk]`.

For a punctuation key, the text is the character of the key on the active
layout. For example, a German keyboard gives `ö`, `#`, `+`, and `<`.

For the other keys, the text is the key name, for example `RControl`. If the key has no
usable character and no name, the text is the virtual-key code.

The program shows only a character that types without a modifier.

```text
key RControl
key <
key vk:226
```

For Shift, Control, and Alt, the program shows only the left or the right key,
for example `LShift`. It does not show the left, right, and middle mouse buttons.

A joystick button has the form `joystick "<device>" button <N>`.

### `bind`

`bind` sets the push-to-talk key or joystick button in the configuration file.
You do not have to edit the file.

1. Run `voicebridge bind`.
2. Wait for the line `press the key or joystick button for push-to-talk (Esc cancels)`.
3. Press the key or the joystick button that you want to use.
4. Read the line that the program prints.
5. Run `voicebridge check`.

The program ignores the inputs that are down when it starts, for example the
Enter key of the command. It decides when you release the key or the button.
Hold the key for a moment. The program can miss a press that is shorter than
20 ms.

Press `Esc` to stop without a change. The exit code is then 1.

- AltGr gives two inputs, `LControl` and `RAlt`. The program writes `RAlt`.
- If you press more than one other input before you release them, the program
  does not use them. It tells you and waits for a new press.

If a key stays down for 1 second, the program shows `waiting for release of:`
and the key.

For a key, the program sets `key` in `[push_to_talk]` and removes `joystick`.
For a joystick button, it sets `joystick` and removes `key`. If the file has no
`[push_to_talk]` table, the program adds the table.

The other lines and the comments of the file do not change. But a comment
directly above the removed line can go away.

Before the program writes the file, it checks that the new file loads. If the
new file does not load, the program gives the reason and keeps the old file.

The program writes a new file and then replaces the old file with it. Thus an
error cannot leave a damaged file.

If the configuration file does not exist, `bind` prints an `error:` line. The
exit code is then 1.

The last line looks like this:

```text
push-to-talk is now key "<" (written to voicebridge.toml)
```

### `commands`

`commands` lists the commands that work with your EDCoPilot files. Use it to
find the phrase for a command.

```text
voicebridge commands
voicebridge commands voice log
voicebridge commands --all
```

Each line shows the phrase, then `->`, then the token. The phrase uses the
VoiceAttack syntax: `[a; b]` means "a" or "b", and `[a;]` means that "a" is
optional. The lines are in groups by category. An alias phrase has `(alias)`
at the end.

Words after `commands` narrow the list. A line stays in the list when the
phrase, the token, or the category has each word.

With `--all`, the list also has the commands that do not work. Each of these
lines gives the reason.

After the commands, the list shows each composite group. The group shows the
rules, the prefix phrases, and then the suffix phrases with their tokens. Say
one prefix and one suffix as one sentence.

Then the list shows the synonyms and the filler words. The filler words are in
one line with `(filler)` at the end. The line shows when you give no filter
words, or when each filter word is in a filler word.

The last line shows how many phrases work, the number of composite groups, the
number of synonyms, and the number of filler words. The exit code is 0.

### `run`

`run` starts the voice session.

```text
voicebridge run
```

To test without a request file, add `--dry-run`. The program then matches your
speech but writes nothing.

```text
voicebridge run --dry-run
```

If the configuration has a problem, `run` prints each `problem:` line and exits
with code 1. Otherwise it prints the start lines (see "Console lines") and then
`listening, Ctrl+C stops`. Press `Ctrl+C` to stop. The exit code is then 0.

After `Ctrl+C`, the program sends no more commands. An utterance that the
speech engine has at that moment can finish. Press `Ctrl+C` again to stop at
once.

If the audio stream fails, `run` prints `audio stream error:` and exits with
code 1.

### `replay`

`replay` sends saved audio files through the speech engine and the matcher. It
is always a dry run. It never writes the request file.

```text
voicebridge replay clips
voicebridge replay clips\utt-20261114T090503042Z-000001.wav
```

The argument is one WAV file or a directory. For a directory, the program uses
all `*.wav` files in name order. Each file must be 16 kHz and mono, with
32-bit float samples or 16-bit integer samples. For another format, the line
shows an `error:` text.

A clip that the program saves now is a 32-bit float file. It holds the samples
that the speech engine got in `run`, bit for bit. Thus `replay` gives the same
text as `run` for this clip.

Clips from older versions are 16-bit files. For such a clip, `replay` can give
a different text, because the model is sensitive to very small changes in the
audio.

For each file, `replay` prints the file name and the result line of `run`. It
uses the speech engine of the configuration. Thus the local engine loads the
model first.

With the cloud engine, `replay` sends each audio file to the service. In
`always_listening` mode, the wake words apply as in `run`.

The exit code is 1 when every file failed. A file fails when the program
cannot read it or when the speech engine gives an error.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | The command finished. For `check`, the configuration is valid. |
| 1 | `check` found a problem, `run` or `replay` found a configuration problem, or the audio stream failed. |
| 1 | In `replay`, every file failed, or the directory has no WAV file. |
| 1 | You pressed `Esc` in `bind`. The file did not change. |
| 1 | The command stopped with an `error:` line on standard error, for example when the file does not exist. |

## Push-to-talk and always-listening

Set the mode with `mode` in `[microphone]`.

In `push_to_talk` mode, the program records while you hold the key or the
button. This is the default mode.

The program records a short time before the press and a short time after the
release. Thus you do not have to hold the key longer than your words. The
program sends the audio when the time after the release ends. Thus this time
adds to the reaction time. Set the two times in `[push_to_talk]`.

If the audio stream stops after the release, the program waits 1 second more.
Then it sends the recording with the audio that it has.

The microphone stream is open during the whole `run` session. The program
keeps the audio from before the press only in memory, for `pre_roll_ms`.

Each recording holds up to `pre_roll_ms` of audio from before the press. The
default is 250 ms and the maximum is 2 seconds. This audio is also in each
saved audio file and in each cloud request. To stop it, set `pre_roll_ms = 0`.

If you press again before the time after the release ends, the program sends
the first recording at once. The new recording starts at the new press.

- The program drops a press that is shorter than 300 ms. Only the time that
  you hold the key counts.
- A recording stops at 15 seconds. The 15 seconds include the time before the
  press and the time after the release. The audio after 15 seconds is not kept.
- Up to 4 recordings can wait for the speech engine. If the queue is full, the
  program drops the new recording. It prints `worker busy, utterance of <N> ms dropped`.

In `always_listening` mode, the program finds the start and the end of your
speech from the sound level. Set `vad_threshold_dbfs` for the level at which
speech starts.

The wake words apply only in `always_listening` mode. They are optional, but
we recommend them: without a wake word, other speech near the microphone can
start a command. When the list has wake words, the text must start with a
wake word. The program ignores case, punctuation, and hyphens. It removes the
wake word and matches the rest. If no wake word is at the start, the program
does not match the text. A wake word alone gives no match. An empty list
means no wake word: the program then matches each text.

In `push_to_talk` mode, the program does not use wake words.

The filler words apply only in `push_to_talk` mode. See "Filler words".

## Use a joystick button for push-to-talk

The fast procedure is `voicebridge bind` (see "`bind`"). To set the button
with an editor, do these steps:

1. Run `voicebridge inputs`.
2. Hold the joystick button that you want to use.
3. Read the line that the program prints.
4. Press `Ctrl+C`.
5. Open `voicebridge.toml`.
6. Remove the `key` line in `[push_to_talk]`.
7. Add a `joystick` line with the device name and the button number.
8. Run `voicebridge check`.

The line for a joystick button looks like this:

```text
joystick "MOZA AB9 Base" button 5
```

A line for a key looks like `key F13`.

Put the device name and the number in the `joystick` line:

```toml
[push_to_talk]
joystick = { device = "MOZA", button = 5 }
```

`device` is a part of the device name. The program ignores case. An empty
`device` matches any joystick. The program uses the first device that matches.

The button number is the raw button code that the operating system reports for
the device. It is not a name such as "South". On Windows it is the index of the
button in the raw button list of the device, counted from 0. Do not guess the
number. Use the number that `inputs` prints.

The joystick button is not proven while the game window has the focus. If the
button does not work in the game, use a keyboard key. You can bind a joystick
button to that key with the software of the joystick.

## Add an alias

An alias is an extra spoken phrase for a command. For example, you can add a
phrase in your own language.

The alias files are `*.toml` files in the folder `aliases_dir`. The default
folder is `core/aliases`. The program loads all `*.toml` files in it.

Each line has this form:

```toml
'OpenVoiceLogPanel' = ["sprachprotokoll [öffnen; zeigen;]"]
```

- The key is the token template of the command, exactly as in the command map.
  Use single quotes, because templates can hold double quotes and braces.
- The value is a list of phrases in VoiceAttack grammar syntax.

To add an alias in the window program, use the tab "Aliases" on the Words
screen. To add an alias with an editor, do these steps:

1. Find the token template of the command: `voicebridge commands --all` shows
   it after `->` on the line of the command.
2. Open a file in `core/aliases`, or make a new `*.toml` file there.
3. Add a line with the token template and the phrases.
4. Save the file.
5. Stop `voicebridge run` and start it again.
6. Run `voicebridge text "<your phrase>"` to test the alias.

If the same token template is in two files, the program joins the phrases.
If a file is not valid TOML, the program skips the whole file without a message.
Use `voicebridge check` to see the number of alias phrases that it loaded.

## Commodity searches

A commodity search has two parts: a prefix and a suffix. Say the two parts as
one sentence. The prefix gives the type of search. The suffix gives the
commodity.

For example, you say "best local sell gold". "best local sell" is the prefix,
and "gold" is the suffix. The program sends one token for the whole sentence.

EDCoPilot has prefixes for more types of search. To see the prefixes and the
suffixes of your installation, run `voicebridge commands`. The composite
groups are at the end of the list (see "`commands`"). `voicebridge commands
gold` shows the phrases with the word "gold".

A prefix alone or a suffix alone is not a commodity search. But some short
forms are other commands of EDCoPilot. If you stop after a prefix, EDCoPilot
can open its own search for that type. A commodity name alone can open the
search panel for the commodity.

When the commodity is one word, it must be an exact match if
`exact_single_word` is `true`. Thus "silvers" in place of "silver" gives a
`rejected` line. The optional words between the prefix and the commodity do
not change this.

The same rule applies to the command "look for <commodity>". The words "look
for" are optional, so the commodity is the only required word. "look for
gold" works. "look for golds" gives a `rejected` line. "look for good" gives
no match.

Nobody has captured these tokens from VoiceAttack. The tokens come from the
actions in the VoiceAttack profile.

## Console lines

### Lines at the start of `run`

| Line | Meaning |
| --- | --- |
| `loaded N commands (...), ... alias phrases` | The load report. The same line is in `check`. |
| `warning: ...` | A path does not exist, for example the voice commands file. |
| `hotwords: N from <file>, score S` | The local engine loaded N hotwords. `hotwords: off` means no hotwords. Only for the `parakeet` engine. |
| `engine: <name>` | The speech engine in use. |
| `mode: push to talk, key "F13" (vk:124)` | The mode and the key as you wrote it, with the virtual-key code that the program reads. For a joystick: `push to talk, joystick "<device>" button <N>`. For the other mode: `always listening`. |
| `input device: <name>` | The microphone. `default input device` means the default device. |
| `threads: N` | The number of threads of the local model. Only for the `parakeet` engine. |
| `dry run: no request file is written` | You used `--dry-run`. |
| `utterance log: <path>` | The program adds a line to this file for each utterance. |
| `audio capture: <dir>` | The program saves the audio of each utterance in this directory. |
| `listening, Ctrl+C stops` | The program is ready. |
| `vocabulary reloaded: N files with M names` | The program reloaded the name lists, after the files in `dynamic_vars_dir` changed. |
| `utterance dropped: waited <N> ms` | An utterance waited more than 5 seconds for the speech engine. The program did not use it, because a late command is a wrong command. |
| `audio glitches (buffer overruns and similar): N` | Printed at the stop. The audio stream lost some samples N times. The session continued. |

### Result lines

Each result line is the result of one utterance. In `run` and `replay`, the line
ends with these values:

```text
(transcribe 120 ms, total 345 ms, clip 2140 ms, peak -6 dBFS, rms -24 dBFS)
```

- `transcribe` is the speech-to-text time. With hotwords, it includes the
  second pass.
- `total` is the time from the end of the recording to the end of the request.
- `clip` is the length of the audio that the speech engine got.
- `peak` and `rms` are the highest level and the mean level of the audio.

In `run`, the recording ends after the time after the release. In `replay`,
the total time starts at the speech-to-text step.

If the peak is -0.1 dBFS or more, the line ends with `clipping`. The audio is
too loud. If the peak is less than -50 dBFS, the line ends with `very quiet`.
The microphone is too far away, or its level is too low. `text` prints no
values.

The line shows `hotwords` when the command came from the second text of the
local engine. See "Hotwords".

In `run`, the line ends with `glitch` when the audio stream reported an error
during the recording. Then the audio can have a gap. A refused real-time
priority also gives the mark, but it loses no audio.

In `push_to_talk` mode, the mark covers the audio from the key press to the
end of the recording. It does not cover the audio from before the press.

| Line | Meaning |
| --- | --- |
| `sent "<text>" -> <token> (0.93)` | The program wrote the token to the request file. The number is the confidence. |
| `dry run "<text>" -> <token> (0.93)` | The text matched, but the program wrote nothing. |
| `no match for "<text>"` | No command matched the text, or the text was empty. |
| `no match for "<text>": blocked: the text is a phrase of a command that the program cannot send` | The text is a phrase of a command that the program cannot send. For example, the command needs a value that the phrase does not give. The program sends no command, also when a different command is near. |
| `no wake word in "<text>"` | In `always_listening` mode, the text did not start with a wake word. |
| `rejected "<text>" -> <token> (0.93): <reason>` | A command matched, but the program did not use it. The reason is `a phrase of one word needs an exact match`. |
| `EDCoPilot did not consume "<text>" -> <token>: withdrawn` | EDCoPilot did not read the token. The program removed the token. |
| `EDCoPilot did not consume "<text>" -> <token>: could not be withdrawn` | EDCoPilot did not read the token. The program could not remove the token. |
| `speech engine error: <message>` | The speech engine failed. The session continues. |
| `send failed for "<text>" -> <token>: <message>` | The program could not write the request file. |

After the confidence, a line can show `(synonym)`. Then the text matched only
after the program replaced your own words with the phrase of EDCoPilot. See
"Use your own words for a phrase".

After the confidence, a line can also show `(sound-alike)`. Then the text
matched only after the program replaced a sound-alike word. See "Sound-alike
words".

After the confidence, a line can also show `(filler)`. Then the text matched
only after the program dropped filler words. See "Filler words".

## Use your own words for a phrase

A synonym is your own word or words for a phrase of EDCoPilot. It works in
every command that uses the phrase. For example, "navigate to" stands for
"set course to". Then "navigate to Sol" works like "set course to Sol".

An alias is different. An alias adds a phrase to one command. A synonym
replaces words in every command.

The file `synonyms` in `[data]` holds the synonyms. Write each line in this
form, under the table `[synonyms]`:

```toml
[synonyms]
"navigate to" = ["set course to", "set course for"]
"navigate" = ["set course for", "set course to"]
```

The shipped file has these two lines and four more lines for forms of
"navigate to" that the engine hears wrong. The key is the phrase that you say.

The shipped file has these 6 entries:

| Key | Value |
| --- | --- |
| "navigate to" | "set course to", "set course for" |
| "navigate" | "set course for", "set course to" |
| "now we get to" | "set course to", "set course for" |
| "navigator" | "set course to", "set course for" |
| "navigate two" | "set course to", "set course for" |
| "navigate too" | "set course to", "set course for" |

The keys "navigate two" and "navigate too" came after a live test, in which
the engine wrote "two" for "to". They have a cost: "navigate two five" as a
coordinate gives no match.

The value is one phrase of EDCoPilot or a list of phrases. Write the words in
lower case. A word can have letters, digits, apostrophes, and hyphens.

The program uses a synonym only when no candidate of the text as heard
reached the threshold. The program also tries the number forms of the text
first.

A text as heard that is not clear stops the search before the synonyms.
Examples are two equally good commands, or a best command whose required
words are not good enough.

- At each place in the text, the longest key wins. Thus "navigate to" wins
  over "navigate".
- Each value gives one form of the text. If two forms give different
  commands, the program gives no match.

A synonym match keeps its confidence. The line shows `(synonym)`. All rules
of the matcher apply to the new text as to a text that you said. Thus
"navigate to hom" gives the same result as "set course for hom".

The synonym also works in other commands with the same phrase. For example,
a command whose phrase starts with "set course for" and then has a number
also accepts "navigate to" and the number.

Do not use words of a command phrase as a key. `voicebridge check` prints a
`warning:` line for such a key. That synonym applies only when no candidate
of the text as heard reached the threshold.

`voicebridge commands navigate` shows the synonyms with that word. If the
program cannot read the file, `check` shows a problem. The program then works
without synonyms.

## Sound-alike words

A speech engine can write a real word that sounds like the command word. For
example, it writes "cell" for "sell" or "by" for "buy".

The file `sound_alikes` in `[data]` holds these pairs. Each line has the form
`heard = "meant"` in lower case, under the table `[sound_alikes]`.

The shipped file has these 13 entries:

| Heard | Meant |
| --- | --- |
| `cell` | `sell` |
| `by`, `bye` | `buy` |
| `soul`, `seoul`, `sole` | `sol` |
| `sail` | `sale` |
| `higher` | `highest` |
| `lock` | `log` |
| `kurs` | `course` |
| `free` | `three` |
| `two`, `too` | `to` |

The entries `two` and `too` came after a live test, in which the engine wrote
"two" or "too" for "to". A phrase with a real "two", for example a ship name
with "mark two", matches as heard. Thus the program does not try the
replacement there.

The program first matches the text as heard, then its number forms. Only when
these find no candidate, it replaces the heard words and tries again. Thus
"by" in place of "buy" in a commodity search gives the command, with the line
mark `(sound-alike)`.

When the text as heard is not clear, the program does not try a replacement.
Examples are two commands that are equally good, or a best command whose
required words are not good enough. Then the result is no match.

- The program multiplies the confidence by 0.95. The result must still reach
  the threshold.
- If two replacements give different commands, the program gives no match.
- A text of one word that matches only after a replacement gives a `rejected`
  line.

The synonym step runs before the sound-alike step. A name from a name list
that is not good enough blocks the search. Thus "soul" after "set course to"
gives no match: "soul" is too far from "Sol", and the program does not try
"sol".

If the replaced word is the only required word or a name of one word from a
name list, the program also gives a `rejected` line. An example is "Seoul"
after "set course to" for the system Sol.

A heard word can be a command word in other phrases, for example "cell" in
the name of a ship module. Such a phrase matches as heard, so the program does
not replace the word. The tests in `core/tests/sound_alikes.rs` check this.

Do not add a word that is itself a command word in the place where you say it.
For example, "sale" is not a heard word in the file, because a command phrase
has the word "sale".

## Filler words

In natural speech, you say small words that the EDCoPilot grammar does not
have. For example, the grammar of a module search has "<search> size 3
<module>", but you say "<search> for a size 3 <module>". The matcher uses all
the spoken words, so the text as heard gives no match.

The file `fillers` in `[data]` holds the filler words. It has two lists:

```toml
not_last = ["a", "an", "the", "for"]
may_be_last = ["uh", "um", "please"]
```

These lists are the default. Both lists are necessary, and a list can be
empty. Write the words in lower case. A word has letters only.

The program does not drop a `not_last` word when it is the last word. A
sentence that stops after "the" or "for" is not complete. Thus a text that
ends with "the" gives no match.

The program can drop a `may_be_last` word at all places. Thus "set course for
home please" works.

The filler step is the last step. The program uses it only when all the other
steps find no candidate: the text as heard, the number forms, the synonyms,
and the sound-alike words.

A text that is not clear stops the search before the filler step. Examples are
two equally good commands, or a best command whose required words are not good
enough.

The program tries all the sets of filler words that it can drop. It also tries
them on the synonym forms and on the sound-alike forms. Thus a synonym, a
sound-alike word, and a dropped filler word can work together in one text.
These rules apply:

- If two sets give different commands, the result is no match.
- A filler word that the phrase needs stays in the set that matches. Thus
  when a command phrase has "for the", the program drops "uh" before the
  phrase and keeps "for the".
- The program drops only words that you said. It does not drop a word that a
  synonym or a sound-alike replacement gave.

These rules keep names and numbers safe:

- The program keeps a filler word in a full name from a name list. Thus for
  a name that starts with "The", "set course to uh the <name>" works, and
  "set course to <name>" fails.
- A dropped "for" or a dropped letter ("a") next to a number or a name gives
  no match. "Next to" means the nearest word on each side that is not a
  filler word. "for" can be "four", and "a" can be a part of a name, for
  example a star name that ends with "A". Thus "music volume for 50" gives no
  match.
- The program also tries "four" in place of a dropped "for". If that gives a
  different command, the result is no match. An example is a module search
  with "for" in the place of the size.

These limits also apply:

- A text of filler words only gives no match.
- A text with more than 3 filler words gets no filler step. More than 32
  variants over all forms of the text also give no filler step.
- When one word stays, that word must be exact. "the help" works.
- The program multiplies the confidence by 0.95, and by 0.95 again after a
  sound-alike word. The result must still reach the threshold.

The line shows `(filler)`. The utterance log shows the mark `filler`.

The filler step is off in `always_listening` mode. In that mode, the program
also hears speech that is not for it. The wake word removes only the first
words. Filler words would let more of such speech match a command.

Do not add a word that can change the command. Examples are "to", "of", "in",
"not", "no", "on", "off", and "dont". Add tests in `core/tests/fillers.rs`
when you add a word.

## Collect data for tuning

The program can keep a log line and the audio of each utterance. Use this data
to find the words that the speech engine hears wrong.

In the window program, use the switches of the Diagnostics screen. For the
console program, do these steps:

1. Open `voicebridge.toml`.
2. In `[diagnostics]`, set `log_file`, for example `"logs/utterances.tsv"`.
3. Set `save_audio_dir`, for example `"clips"`.
4. Make the directory of `log_file`.
5. Run `voicebridge check`.
6. Run `voicebridge run` and use the program as usual.

The program adds one line to the log for each utterance. Tab characters
separate the fields:

| Field | Example |
| --- | --- |
| Time (UTC, ISO 8601) | `2026-11-14T09:05:03.042Z` |
| Result | `sent`, `dry_run`, `no_match`, `no_wake_word`, `rejected`, `not_consumed`, `engine_error`, `send_error`, or `dropped` |
| Heard text | `Look for gold.` |
| Token | The token of the command, or empty |
| Confidence | `0.95`, or empty |
| Speech-to-text time in ms | `61` |
| Total time in ms | `75` |
| Audio file name | `utt-20261114T090503042Z-000001.wav`, or empty |
| Peak level in dBFS | `-6.0` |
| RMS level in dBFS | `-24.4` |
| Marks | `synonym`, `sound_alike`, `filler`, `hotwords`, `clipping`, `very_quiet`, `glitch`, a `blocked_` mark, separated by spaces, or empty |

The heard text is the text after the program removed the wake word.

A `no_match` line can have one `blocked_` mark. It shows why the program sent
no command, although a command was near:

- `blocked_tie`: two commands have the same score.
- `blocked_required_words`: a word or a name that the best command needs is
  not near enough.
- `blocked_no_token`: the text is a phrase of a command that the program
  cannot send.
- `blocked_forms`: two forms of the text give different commands.
- `blocked_dropped_word`: the program removed a short word or a word that
  sounds like a number next to a number or a name.

Lines from an older version have fewer fields at the end. The other fields are
in the same order.

The audio files are 16 kHz mono WAV files with 32-bit float samples. They
need approximately 2 times the disk space of the older 16-bit files. A file
of 2.5 seconds is approximately 160 KB, and 500 files are approximately
80 MB. The names sort by time. The program
keeps the newest 500 files and deletes the older files.

The program deletes only files with the name form that it makes. It never
deletes the file that it just wrote. Use a directory that holds nothing else.

If the program cannot write the log or an audio file, it prints one warning.
Then it stops that part of the collection. The session continues. If the
program cannot delete an old audio file, it prints one warning and continues
the collection.

To measure a change against your saved audio, run `voicebridge replay clips`.
Compare the lines before and after the change.

The audio files hold your voice. They stay on your computer. The program does
not send them to a service, except the normal speech-to-text request of the
cloud engine.

In `push_to_talk` mode, each audio file and each cloud request holds up to
`pre_roll_ms` of audio from before the key press. The default is 250 ms and
the maximum is 2 seconds. To switch this off, set `pre_roll_ms = 0`.

The log holds no API key and no other configuration value. Delete the files
when you do not need them.

## Rules of the matcher

- A wrong command is worse than no command. The matcher gives no result when
  two different commands are equally good.
- The matcher accepts a phrase only when the phrase uses all the spoken words.
- When `exact_single_word` is `true`, a phrase of one word needs an exact match.
  A close match gives a `rejected` line. Set it to `false` to accept close
  matches. Then a small error can give a wrong command.
- Optional words do not hide an error in the other words. The words outside
  optional groups (`[look for;]`) must reach the threshold alone. If these
  words are one word, that word needs an exact match when
  `exact_single_word` is `true`.
- A name from a name list, for example a system after "set course to", must
  reach the threshold alone. Each word of the name must also reach the
  threshold. Thus a name of two words with one wrong word does not give the
  name.
- When `exact_single_word` is `true`, a system name or a commodity name of one
  word must be heard exactly. After "set course to", this applies to the
  names from the vocabulary files and to the fixed names of the engineers
  and titans.
- An engineer or titan command also accepts its system name. The name of the
  engineer and the name of the system give the same token. `voicebridge
  commands` shows both names in one group, for example `[<engineer>;
  <system>]`.
- When two commands are equally good and reach the threshold as heard, another
  form of the text does not decide. The program does not try number forms or
  sound-alike words then. A text as heard below the threshold has no candidate.
- Filler words ("uh", "the", "please") can go only when all other forms of
  the text find no candidate. See "Filler words".
- A sound-alike candidate that is below the threshold after the factor 0.95
  stops the search. A text as heard below the threshold does not stop it,
  because the matcher does not tell how close that text came.
- Number words become digits. "music volume fifty five" gives the token
  `UpdatePrefs:MusicVolume="55"`. The forms "fifty-five" and "12,000" also work.
- Two or more single digits in a row become one number, but only in a name.
  Thus "set course to <name> one zero eight" works like "set course to <name>
  108", for a system name with digits. "music volume five five" gives no
  match. The word "oh" does not count as zero.
- The program also tries the digits one by one, for example "3 4". If two
  forms give different commands, the program gives no match.
- A number with a zero at the start, for example "05", is no value. The fixed
  zero of a phrase still works: a phrase with the word "zero" before a number
  gives "05" in the token.
- The matcher ignores case and punctuation in the text.
- After the program writes a token, it waits until EDCoPilot reads the file. If
  EDCoPilot does not read it in 2 seconds, the program removes the token and
  prints the `did not consume` line. Thus an old command cannot run when
  EDCoPilot starts later.

## Limits

- EDCoPilot must be in operation. If EDCoPilot is not in operation, the program
  cannot send a command.
- EDCoPilot needs 1 to 2 minutes to start.
- EDCoPilot stops itself after 15 minutes without an Elite Dangerous session.
- EDCoPilot reads the request file about every 500 ms.
- 151 number commands and 200 bookmark group commands cannot run. They need a
  bookmark group name or a preference value.
- The program does not read `EDCoPilot.VoiceCommandsCustom.ini`. Your own
  commands in that file do not work.
- Some commands of the profile are not in the command map and do not work.
  `voicebridge check --left-out` lists them with the reason:
  - Commands that write no token. These are VoiceAttack commands, for example
    the commands that stop VoiceAttack or change how it listens.
  - Commands that write two or more tokens.
  - VoiceAttack maintenance commands, for example the commands that change
    the EDCoPilot profile in VoiceAttack.
- Nobody has confirmed the exact token text for the commands whose token
  holds the spoken command text (`{CMD}`).
- The command "move the fleet carrier to <system>" can match. It is not known
  if EDCoPilot asks for confirmation. Be careful in `always_listening` mode.
- A single word with a small error can match at the 0.8 threshold. Keep
  `exact_single_word` set to `true`.
- The program loads the aliases when a session starts. Restart the session
  after you change an alias file.
- Most keyboards have no `F13` key. Use "Press to set" on the Microphone
  screen, or run `voicebridge bind`, and press a key that you have. You can also bind `F13` to a joystick button with the software of
  the joystick.
- In `always_listening` mode with the cloud service, the program sends each
  utterance that it hears to the service. It examines the wake word after that.
- If the default microphone of Windows changes during a session, the session
  stops. Start it again.
