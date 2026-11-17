# Release checklist

This file is the manual checklist for a release. A person does the steps on
a real computer with the installer of the release. The automatic tests cannot
do these steps. Sections A to L check the window program `voicebridge-ui`.
Section M checks the console program `voicebridge`. Section N checks the
update inside the program. It needs an older installed version.

Do the check in a copy of this file. Write the result of each step in the
column "Result": `pass`, `fail`, or a short note. Do not commit the results.

## Rules for the check

- Do the steps in their order. A later step can need the result of an earlier
  step.
- **Real command** at the start of a step means that the step sends a command
  to EDCoPilot. Do these steps only when EDCoPilot is in operation, and only
  with a command that does no harm, for example "help voice log".
- Some steps use example phrases. If a phrase is not in your EDCoPilot
  installation, use a similar phrase from the Commands screen.
- For each other step with speech, use "Start as dry run".
- Do not write a key, a user name, or a device name of your computer into the
  results.

## Before the check

1. Get the installer of the release. To build an installer from the current
   code, see [BUILDING.md](BUILDING.md).
2. Section A starts with a clean install. Make sure that the computer has no
   folder `%APPDATA%\VoiceBridge`. If the folder exists, rename it. Step L2
   checks the update of an older version: it makes the state of an older
   version by hand, so you need no older installer.
3. Make sure that the computer has EDCoPilot.
4. Connect a keyboard, a microphone, and a joystick.
5. Find the token of the command "help voice log" on the Commands screen, or
   with `voicebridge commands voice log`. Steps E3, F2, and G4 compare the
   result with tokens.

## A. Installer and first start

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| A1 | Start the installer file `EDCoPilot VoiceBridge_<version>_x64-setup.exe`. | Windows SmartScreen shows a warning, because the installer has no code signature. | |
| A2 | Click "More info", then click "Run anyway". | The installer starts. Windows asks for no administrator rights. | |
| A3 | Do the steps of the installer. | The folder `%LOCALAPPDATA%\EDCoPilot VoiceBridge` holds `voicebridge-ui.exe` and the folder `resources`. | |
| A4 | Start EDCoPilot VoiceBridge. | The window opens with the Setup screen. The screen shows "Start is off until you fix these problems". | |
| A5 | Open `%APPDATA%\VoiceBridge` in Explorer. | The folder holds `voicebridge.toml`, `synonyms.toml`, `sound_alikes.toml`, `fillers.toml`, `hotwords.txt`, and the folder `aliases`. | |
| A6 | Read "EDCoPilot folder" on the Setup screen. | The field shows `C:\EDCoPilot` when that folder exists. Else the field is empty. | |

## B. Setup screen

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| B1 | Click "Browse" beside "EDCoPilot folder". | A folder dialog opens. | |
| B2 | Select the EDCoPilot folder. | The field shows the folder. The navigation shows "unsaved" beside Setup. | |
| B3 | Click "Save". | The screen shows "Saved." | |
| B4 | Read the text beside "Download the model". | The text is "487 MB download, 671 MB on the disk". | |
| B5 | Click "Download the model". | A progress bar moves. The text starts with "Downloading:". | |
| B6 | At approximately 30 percent, click "Cancel". | The screen shows "The download stopped. A new download continues from the same point." | |
| B7 | Open `%APPDATA%\VoiceBridge\models` in Explorer. | The folder has a `.part` file and no model folder. | |
| B8 | Click "Download the model" again. | The progress bar starts at approximately the point of the cancel, not at 0. | |
| B9 | Wait until the download ends. | The screen shows "Checking the downloaded file.", then "Unpacking the model.", then "The model is in <folder>." | |
| B10 | Read the top of the Setup screen. | The screen shows "The check found no problem." The `.part` file is gone. | |
| B11 | Set "Threads" to 0, then click "Save". | The field shows a problem. The file keeps the old value. | |
| B12 | Click "Discard changes". | "Threads" shows the saved value again. | |
| B13 | Keep the window open. Open `%APPDATA%\VoiceBridge\voicebridge.toml` in a text editor. Set `threshold` in `[matching]` to `0.75` and save the file. | The window does not change yet. | |
| B14 | In the window, set "Threshold" to `0.85`. Click "Save". | The screen shows "Not saved." with "the configuration file changed, read it again: ...". The link "Read the file again and keep my changes" shows. The file still has `0.75`. | |
| B15 | Click "Read the file again and keep my changes". | "Threshold" still shows `0.85` as an unsaved change. | |
| B16 | Click "Save". | The screen shows "Saved." The file has `0.85`. | |
| B17 | In the text editor, set `threads` in `[stt.parakeet]` to another value and save the file. In the window, change "Threshold" and click "Save". | The screen shows "Saved." The file has the new threshold and the `threads` value of the text editor. | |

## C. API key

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| C1 | Under "Engine", select "Cloud (OpenAI-compatible)", then click "Save". | The screen shows "Saved." The key state is "No key is saved", or it names the environment variable. | |
| C2 | Type a valid API key in "Key", then click "Save key". | The screen shows "The key is saved." The key state is "A key is saved for <server>". The field is empty. | |
| C3 | Open `%APPDATA%\VoiceBridge\voicebridge.toml` in a text editor and search for the key. | The file does not hold the key. | |
| C4 | Open the Windows Credential Manager. | It has an entry with the service name "VoiceBridge". | |
| C5 | In the tray menu, click "Quit". Then start the program again. | The key state is "A key is saved for <server>". | |
| C6 | On the Status screen, click "Start as dry run". Speak one command. | The result row shows the heard text from the cloud service. | |
| C7 | Click "Stop". Change "Server address" to another server, but do not save. | The screen shows "The saved key belongs to <server>. After the save, enter the key again for <server>." The key field is off. | |
| C8 | Click "Discard changes". | The warning goes away. The key field is on. | |
| C8a | Click "Start as dry run". While the session runs, type the key again in "New key" and click "Replace key". | The screen shows "The key is saved. The running session keeps the old key until you stop and start it." The note about the restart shows. | |
| C8b | Click "Stop". | The note about the restart goes away. | |
| C9 | Click "Remove key", then click "Remove". | The screen shows "The key is removed." The key state is "No key is saved", or it names the environment variable. | |
| C10 | Under "Engine", select "Local (Parakeet)", then click "Save". | The screen shows "Saved." | |

## D. Microphone screen and the talk button

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| D1 | Open the Microphone screen. Read "Device". | The list has "Default device" and the connected microphones. | |
| D2 | Click "Press to set". Press and release F8. | The talk button shows "F8". The screen shows "The talk button is now F8. Save to keep it." | |
| D3 | With a German keyboard layout, click "Press to set". Press and release the key `<`. | The talk button shows "<". | |
| D4 | Click "Press to set". Press Esc. | The screen shows "Cancelled. The talk button did not change." | |
| D5 | Click "Press to set". Press and release a joystick button. | The talk button shows "<device> button <N>". | |
| D6 | Click "Press to set". Wait 30 seconds without a press. | The screen shows "Not set: no key or button was pressed in 30 seconds". | |
| D7 | Set the talk button that you use in the game, then click "Save". | The screen shows "Saved." | |
| D8 | On the Status screen, click "Start as dry run". Open the Microphone screen. | "Press to set" is off. The text is "Stop the session to set the talk button. The session reads the talk button." | |
| D9 | On the Status screen, click "Stop". | The bar shows "Stopped". | |

## E. Status screen and sessions

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| E1 | Click "Start as dry run". | The bar shows "Running" and "Dry run: nothing is sent". The panel "This session" shows the start lines. | |
| E2 | Hold the talk button. | The lamp shows "Talk button down: listening". | |
| E3 | Speak "help voice log", then release the talk button. | A row "Dry run" shows the token of "help voice log". EDCoPilot does nothing. | |
| E4 | Speak "what is the weather". | A row "No match" shows. | |
| E5 | Click "Stop". | The bar shows "Stopped". The panel title is "Last session". | |
| E6 | Start EDCoPilot and wait until it is ready. | EDCoPilot is in operation. | |
| E7 | **Real command.** Click "Start". Speak "help voice log". | A row "Sent" shows. EDCoPilot opens the help for the voice log. | |
| E8 | **Real command.** Close the window. Put the game in the foreground. Speak a command with the talk key. | EDCoPilot does the command. The window stays hidden. | |
| E9 | Set a joystick button as talk button. Start a dry run. Put the game in the foreground. Speak with the button. | After the step, the Status screen shows a result row for the utterance. | |
| E10 | During a running session, change "Threshold" on the Setup screen, then click "Save". | The screen shows "The running session keeps its old settings until you stop and start it." | |
| E11 | On the Status screen, click "Stop", then click "Start as dry run". | The session starts. The note of step E10 goes away. | |
| E12 | During a dry run, disconnect the microphone. | The bar shows "Stopped by error" and a reason that starts with "Microphone error". | |
| E13 | Connect the microphone again. Click "Start as dry run". | The session starts. | |
| E14 | Click "Stop". Set "Threshold" back to the value before step E10, then click "Save". | The screen shows "Saved." | |

## F. Always-listening

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| F1 | On the Microphone screen, select "Always listening", then click "Save". | The screen shows the wake words "copilot" and "co pilot". | |
| F2 | Start a dry run. Speak "copilot, help voice log". | A row "Dry run" shows the token of "help voice log". | |
| F3 | Speak "help voice log" without the wake word. | A row "No wake word" shows. | |
| F4 | Click "Stop". Remove the two wake words on the Microphone screen. | Before the save, the screen shows the warning "Always-listening without a wake word sends each match: ...". | |
| F5 | Click "Save". Open the Status screen. | The Status screen shows the same warning and the link "Open Microphone". | |
| F6 | Start a dry run. Speak "help voice log" without a wake word. | A row "Dry run" shows. This is the risk of the warning. | |
| F7 | Click "Stop". Add the wake words "copilot" and "co pilot" again. | The warning goes away. | |
| F8 | Select "Push-to-talk", then click "Save". | The screen shows "Saved." The Status screen shows no warning. | |

## G. Commands screen

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| G1 | Open the Commands screen. | The list shows commands in groups by category. The last line is the summary. | |
| G2 | Type "voice log" in the filter field. | Only rows with "voice" and "log" stay. | |
| G3 | Select "Show commands that do not work". | Rows that cannot run show their reason. | |
| G4 | Under "Try a phrase", type "set course for home", then click "Test". | The result is "Dry run" with a token. Note the token. | |
| G5 | **Real command.** Start a session (not a dry run). Test "set course for home" again. | The result is "Dry run". EDCoPilot does nothing. | |
| G6 | **Real command.** Speak "set course for home" in the same session. | A row "Sent" shows. The token is the token of step G4. | |
| G7 | Click "Stop". Select "Always listening" and save. Test "help voice log". | The result is "No wake word". The screen names the wake words. | |
| G8 | Select "Push-to-talk", then click "Save". | The screen shows "Saved." | |

## H. Words screen

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| H1 | Open the Words screen. Click "Synonyms". | The line "File:" names `synonyms.toml` in `%APPDATA%\VoiceBridge`. | |
| H2 | Start a dry run. On the Words screen, click "Add". | A new empty row shows. | |
| H3 | Type "head for" in "Your words" and "set course for" in "EDCoPilot phrases". | A warning shows when a command phrase starts with "head for". "Save" is off until you confirm the warnings. If no warning shows, "Save" is on. | |
| H4 | Click "I read the warnings: save is permitted", then click "Save". | The screen shows "Saved. The running session keeps the old table until you stop and start it again." | |
| H5 | On the Commands screen, test "set course for sol", then test "head for sol". Use a system of your name lists in place of Sol when necessary. | The two results are "Dry run" with the same token. The second result has the mark "synonym". | |
| H6 | Speak "head for sol" in the running dry run. | The row does not show the token of H5. The session keeps the old table. | |
| H7 | Click "Stop", then click "Start as dry run". Speak "head for sol". | A row "Dry run" with the token of H5 shows. | |
| H8 | Click "Stop". On the "Hotwords" tab, add the rows "Lanterna" and "Lanternia". | A problem names the two words. "Save" stays off. | |
| H9 | On the "Filler words" tab, add the row "uh2". | A problem shows. "Save" stays off. | |
| H10 | Click "Discard changes and read the file again" on each changed tab. | The navigation shows no "unsaved" beside Words. | |
| H11 | In a text editor, add the line `[unknown]` to `fillers.toml` in the data folder. | The "Filler words" tab shows a text that ends with "edit the file by hand". | |
| H12 | Remove the line `[unknown]` again. | The tab shows the rows again. | |

## I. Diagnostics screen

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| I1 | Open the Diagnostics screen. | The two switches are off. "Data folder" shows `%APPDATA%\VoiceBridge` and the text about the uninstall. | |
| I2 | Switch on "Utterance log". | The screen shows the path of `logs\utterances.tsv` and "Open folder". | |
| I3 | Click "Open folder" of the utterance log. | Explorer opens the folder `logs`. | |
| I4 | Switch on "Audio capture". | The screen shows the path of `logs\audio` and the text "The files hold your voice and stay on this computer." | |
| I5 | Click "Open folder" of the audio capture. | Explorer opens the folder `logs\audio`. | |
| I6 | Click "Open folder" of "Data folder". | Explorer opens `%APPDATA%\VoiceBridge`. | |
| I7 | Start a dry run. Speak one command. Click "Stop". | `utterances.tsv` has a new line. `logs\audio` has a new WAV file. | |
| I8 | Switch off the two switches. | The paths go away. | |
| I9 | Read "Output of the check". | The last line is "configuration is valid". | |

## J. Notification area

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| J1 | Click the close button of the window for the first time in this run. | A message says "VoiceBridge keeps running in the notification area. ...". After "OK", the window hides. | |
| J2 | Click the VoiceBridge icon in the notification area. | The window shows. | |
| J3 | Click the close button again. | The window hides without a message. | |
| J4 | Point to the icon. | The tooltip is "VoiceBridge: stopped". | |
| J5 | Right-click the icon. | The menu has "Show VoiceBridge", "Start...", "Stop", and "Quit". "Stop" is off. | |
| J6 | Click "Show VoiceBridge". | The window shows. | |
| J7 | Hide the window. In the menu, click "Start...". | The window shows the Status screen. No session starts. | |
| J8 | Rename the model folder in Explorer. In the menu, click "Start...". | The window shows the Setup screen with a problem for the model folder. | |
| J9 | Rename the model folder back. Click "Check again". | The screen shows "The check found no problem." | |
| J10 | Start a dry run. Point to the icon. | The tooltip is "VoiceBridge: dry run, nothing is sent". | |
| J11 | In the menu, click "Stop". | The session stops. "Stop" in the menu is off again. | |
| J12 | Start a dry run. In the menu, click "Quit". | The program ends. The icon goes away. | |

## K. One instance, logoff, update, and uninstall

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| K1 | Start the program. Hide the window. Start the program a second time. | The first window shows. The notification area has one VoiceBridge icon. | |
| K2 | Start a dry run. Hide the window. Log off from Windows. | Windows logs off. | |
| K3 | Log on. Start the program. Click "Start as dry run". | The session starts. No problem names a session that is already running. | |
| K4 | Change one synonym on the Words screen and save. Install the same installer again. | After the start, the Words screen shows your changed synonym. | |
| K5 | Uninstall the program. Select "Delete the application data". | The folder `%LOCALAPPDATA%\EDCoPilot VoiceBridge` is gone. | |
| K6 | Open `%APPDATA%\VoiceBridge` in Explorer. | The folder still holds the configuration, the word tables, and the model. | |

## L. Commands from the EDCoPilot profile

The program ships no command map. It reads the commands from the EDCoPilot
profile `EDCoPilot-Profile.vap` in the EDCoPilot folder.

Do not change a file in the EDCoPilot folder. Some steps use a copy of the
profile in a scratch folder: L7 makes the copy, and the steps L13 to L18
use it again.

Before L1: you installed the program, and the data folder
`%APPDATA%\VoiceBridge` is the folder of section A. If you uninstalled the
program in section K, install it again with the same installer. The
installer does not change the data folder.

In these steps, `voicebridge.toml` is the file
`%APPDATA%\VoiceBridge\voicebridge.toml`. The program folder is the full
path of `%LOCALAPPDATA%\EDCoPilot VoiceBridge`, for example
`C:\Users\<name>\AppData\Local\EDCoPilot VoiceBridge`.

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| L1 | Open `voicebridge.toml` in a text editor. Open the folder `resources\data` of the program folder in Explorer. | The file has no line `command_map` and no line `profile_vars`. The folder has no file `command_map_v3.json` and no file `profile_vars.json`. | |
| L2 | Make the state of an older version by hand. Quit the program (tray menu, "Quit"). Keep a copy of `voicebridge.toml` to compare later. In `voicebridge.toml`, below `[data]`, replace the line `# The alias folder.` with the three lines `# The command map, the profile variables, and the alias folder.`, `command_map = '<program folder>\resources\data\command_map_v3.json'`, and `profile_vars = '<program folder>\resources\data\profile_vars.json'`. Keep the line `aliases_dir` directly below them. Save the file. Start the program. | The Status screen shows "The program reads the commands from the EDCoPilot profile now. The update removed data.command_map and data.profile_vars from the configuration." The file has no line `command_map` and no line `profile_vars`. The line above `aliases_dir` is `# The alias folder.` again. The file is the same as your copy. | |
| L3 | Quit the program and start it again. | The Status screen does not show the note of L2. The file did not change. | |
| L4 | Quit the program. Below `[data]` in `voicebridge.toml`, add the line `command_map = 'D:\mine\map.json'`. Start the program. | The Status screen shows the note "data.command_map is not used any more. The program reads the commands from the EDCoPilot profile. You can remove the line from the configuration." The Setup screen shows the warning "data.command_map and data.profile_vars are not used any more, the program reads the EDCoPilot profile". The line stays in the file. | |
| L5 | Quit the program. Remove the line of L4 from `voicebridge.toml`. Start the program. Open the Setup screen. | Below "EDCoPilot folder", the screen shows "The program reads the commands from this profile: C:\EDCoPilot\EDCoPilot-Profile.vap" (with your EDCoPilot folder). | |
| L6 | Open the Diagnostics screen. Open "Every left-out command, with its reason". | "Commands from the EDCoPilot profile" shows "commands from the EDCoPilot profile: <n> commands, <m> left out", and one line "left out, <n> commands: ..." for each reason. "Output of the check" has the same lines after the line "loaded ...". The list shows "<m> commands. 200 shown." Each line of the list starts with `left out: "`. "Show more" adds the next 200 lines, until all lines show. A filter word shows only the lines with that word. | |
| L7 | Copy `EDCoPilot-Profile.vap` from the EDCoPilot folder to a scratch folder, for example `C:\Scratch\EDCoPilot-Profile.vap`. In `voicebridge.toml`, below `[edcopilot]`, add the line `profile = 'C:\Scratch\EDCoPilot-Profile.vap'`. Click "Check again" on the Setup screen. Click "Start as dry run". Open the copy in a text editor, replace all of its text with `not a profile`, and save it. Wait 10 seconds. | The session keeps running. On the Status screen, "Warnings" has the line "EDCoPilot changed its profile, stop and start the session to use the new commands". | |
| L8 | Click "Stop". Click "Start as dry run". | The session does not start. Under "The session did not start", the screen shows "the file <path of the copy> is not an EDCoPilot profile that the program understands: ...". The text ends with an action, for example "use the profile file of EDCoPilot". | |
| L9 | In `voicebridge.toml`, change the line `profile` to `profile = 'C:\Scratch\absent.vap'`. Open the Setup screen and click "Check again". | Below "EDCoPilot folder", the screen shows "Problem: cannot read the EDCoPilot profile C:\Scratch\absent.vap: the file does not exist, set edcopilot.profile to the profile file". The list of problems at the top has the same text. The Start buttons are off. | |
| L10 | Open the Commands screen. | The screen shows "The command list did not load" with the same text. It shows no empty list. | |
| L11 | Open the Diagnostics screen. | "Commands from the EDCoPilot profile" shows the same problem text and no report. | |
| L12 | Remove the line `profile` from `voicebridge.toml`. Click "Check again". Click "Start as dry run", then "Stop". | The check finds no problem. The session starts. | |
| L13 | Copy `EDCoPilot-Profile.vap` from the EDCoPilot folder over the copy of L7 again. Set `profile = 'C:\Scratch\EDCoPilot-Profile.vap'` in `voicebridge.toml` again. Click "Check again". Click "Start as dry run", then "Stop". | The session starts. "Warnings" on the Status screen has no line "the EDCoPilot profile changed". The last start used the profile of the EDCoPilot folder: the program does not compare two different profile files. | |
| L14 | Change the commands in the copy in one of two ways. Way 1: copy an older `EDCoPilot-Profile.vap` (for example from a backup) over the copy. Way 2: in VoiceAttack, import the copy as a new profile and delete one command with a phrase. Export the profile over the copy. Then delete the imported profile in VoiceAttack. After the change, click "Check again" on the Diagnostics screen. | "Commands from the EDCoPilot profile" and "Output of the check" show the line "the EDCoPilot profile changed: <a> commands added, <r> removed, <l> now left out, <m> now in the map". The screen can show "is not an EDCoPilot profile that the program understands" in its place. Then the export of VoiceAttack has another format: write the text in "Result". | |
| L15 | Click "Start as dry run". | On the Status screen, "Warnings" has the same line "the EDCoPilot profile changed: ...". | |
| L16 | Click "Stop". Click "Check again" on the Diagnostics screen. | The line "the EDCoPilot profile changed" is gone. The start of L15 saved the report of the changed copy, and the check compares with that report. | |
| L17 | On the Diagnostics screen, open "Every left-out command, with its reason". Click "Check again" on the Diagnostics screen. | The open list loads again: the count line and the rows come back with the values of the new check, and the filter stays. | |
| L18 | Copy `EDCoPilot-Profile.vap` from the EDCoPilot folder over the copy again. Click "Check again" on the Setup screen. | Below "EDCoPilot folder", the screen shows the path of the copy. Remove the line `profile` from `voicebridge.toml` and click "Check again": the screen shows the profile of the EDCoPilot folder. | |
| L19 | After a real update of EDCoPilot, click "Check again" on the Diagnostics screen before you start a session. Then start a dry run. | When the update changed the commands, the check and the warnings of the start show the line "the EDCoPilot profile changed: ...". When the update did not change the commands, no line shows. | |

## M. Console program

Build the console program (see [BUILDING.md](BUILDING.md)). Make a
configuration file for it (see "Make the configuration file" in
[USAGE.md](USAGE.md)). Use the push-to-talk mode with the local engine.

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| M1 | Run `voicebridge check`. | The output has the load report and the line "configuration is valid". The exit code is 0. | |
| M2 | Run `voicebridge devices`. | The list has your microphone. | |
| M3 | Run `voicebridge text "help voice log"`. | The output is a "dry run" line with the token of "help voice log". | |
| M4 | Run `voicebridge text "what is the weather"`. | The output is a "no match" line. | |
| M5 | Run `voicebridge inputs`. Press the talk key, then press `Ctrl+C`. | The output shows the key. | |
| M6 | Run `voicebridge run --dry-run`. Hold the talk key and speak "help voice log". Press `Ctrl+C`. | A "dry run" line shows the token of "help voice log". | |
| M7 | **Real command.** Start EDCoPilot. Run `voicebridge run`. Speak "help voice log". Press `Ctrl+C`. | A "sent" line shows. EDCoPilot opens the help for the voice log. | |
| M8 | Set a joystick button as talk button with `voicebridge bind`. Run `voicebridge run --dry-run`. Click into another window, for example the game. Hold the joystick button and speak a command. | A result line shows. The button works without focus. If it does not work, write `fail`: the keyboard key is then the talk button for the game. | |
| M9 | Set `mode = "always_listening"` in `[microphone]`. Run `voicebridge run --dry-run`. Speak "copilot, help voice log", then "help voice log". | The first text gives a "dry run" line. The second text gives a "no wake word" line. | |

## N. Updates inside the program

Do this part at the release of a version after 0.2.0. It needs two
releases: an older installed version with the updater (0.2.0 or later), and
the new version as a published release with `latest.json`. A draft release
does not reach the program. Thus do N3 to N12 after you publish the release.
Do N1 and N2 before you publish.

After you publish, wait 5 minutes before N3. For some minutes, GitHub can
still send the program to the older release. Then the program shows the
older state, or "the server did not send the update information".

| # | Action | Expected result | Result |
| --- | --- | --- | --- |
| N1 | Open the draft release on GitHub. Open the run of the workflow. | The run has the job "Sign the installer for the updater" in the environment `release`. The draft has the installer, `<installer>.sig`, `SHA256SUMS.txt`, and `latest.json`. `latest.json` has the new version without "v", the text of the CHANGELOG section as `notes`, and the address of the installer of this release. | |
| N2 | On the Setup screen of the installed older version, clear "Check for updates at the start and one time each day" and click "Save". Quit the program and start it again. Wait 30 seconds. | No note shows on the Status screen. The Diagnostics screen shows "The program does not check for updates (updates.check is false)." | |
| N3 | Publish the release. On the Diagnostics screen, click "Check for updates". | The line shows "Version <new> is available." The Status screen shows "Version <new> is available. You have version <old>." | |
| N4 | Select "Check for updates at the start and one time each day" again and click "Save". Quit the program and start it again. Wait 30 seconds. | The note shows on the Status screen without a click. | |
| N5 | Click "Later". Open another screen and come back. | The note is gone. It does not come back before the next start or the next click on "Check for updates". | |
| N6 | Click "Check for updates" on the Diagnostics screen. Open the Status screen. Open "What is new". | The note shows again. "What is new" shows the changes of the new version and the address of its release page. | |
| N7 | Click "Start as dry run". | "Install update" is off. The note shows "Stop the session first." | |
| N8 | Click "Stop". Click "Install update". Click "Cancel" during the download. | The note shows "Downloading the update: <n> of <m> MB." After "Cancel", the note shows the offer again. The program does not change. | |
| N9 | Click "Install update". During the download, click "Start". | The session does not start. The screen shows "The update is in progress." | |
| N10 | Wait. | The note shows "The update is ready. The program closes and the installer starts." The program closes. A small installer window shows progress and asks no questions. No Windows administrator prompt shows. | |
| N11 | Wait until the installer ends. | The program starts again by itself. The version in the window is the new version. The Diagnostics screen shows "You have the latest version. Last check: <time>." after a click on "Check for updates". | |
| N12 | Open the Setup and Microphone screens. Click "Start as dry run" and speak a command. | Your settings, the talk button, the word tables, and the API key are the same as before the update. The session works. | |

## After the check

1. Write the date, the version, and the result of each step in your copy of
   this file.
2. For each `fail`, write a short note with the text that the window showed.
3. If you renamed an old data folder before the check, give it its old name
   again.
