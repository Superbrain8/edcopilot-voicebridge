# The EDCoPilot profile format and the map builder

This document is for a developer who maintains `core/src/profile/`. Read it
before you change the builder, and after EDCoPilot or VoiceAttack has a new
version.

The program builds its command map from the EDCoPilot profile on the computer
of the user, at each load of the commands. The profile is the VoiceAttack
profile that EDCoPilot installs: `<EDCoPilot folder>\EDCoPilot-Profile.vap`.
The repository has no data from this profile. The tests use an invented
profile (see "Test strategy").

## The main rule

A wrong match is worse than a miss. When the builder does not fully
understand a command, it leaves that command out with a reason. When the file
is not a profile that the builder can read safely, the build stops with a
typed error. The builder never guesses.

The rule of a record is "the same token as VoiceAttack". A record goes into
the map only when the program sends the same request token that VoiceAttack
writes for the same spoken text.

## Where the code is

| Item | Path |
|---|---|
| Entry point `build_map(&[u8]) -> Result<BuiltMap, BuildError>` | `core/src/profile/mod.rs` |
| Inflate step, record walk, field signatures, tested settings | `core/src/profile/record.rs` |
| Extraction rules, classes, profile variables, name patterns | `core/src/profile/extract.rs` |
| Action table and whitelist | `core/src/profile/allow.rs` |
| Typed errors and their texts | `core/src/profile/error.rs` |
| Report of the build (facts, left-out commands, counts) | `core/src/profile/report.rs` |
| Writer of synthetic profiles (tests only) | `core/src/profile/test_support.rs` |
| Invented profile (tests only) | `core/src/profile/fixture.rs` |
| Load of the profile from the EDCoPilot folder | `app/src/commands_source.rs` |
| Timing and format example | `core/examples/profile_build.rs` |

The library part is pure. `build_map` reads no file and writes no file.
`BuiltMap` holds these items:

- `commands`: the records, with the type `Command` of
  `core/src/command_map.rs`.
- `profile_vars`: the profile variables (variable name to grammar).
- `report`: the facts of the source (SHA-256, sizes, counts), the format
  note, and the left-out commands with a class and a reason.
- `blocking`: the phrases of the left-out commands that VoiceAttack
  recognises by voice (see "Blocking phrases").

The dependencies are `flate2` with the feature `rust_backend` only (pure Rust,
no C library) and `sha2`. The versions are exact in `core/Cargo.toml`.

## The container

The `.vap` file is one raw DEFLATE stream, with no header. The builder
requires that the stream ends at the last byte of the file. The inflated data
is a tree of records.

Each record has this form. Each number is a little-endian 32-bit integer.

```text
[i32 size] [i32 n] [i32 offset x (n + 1)] [field 0] ... [field n]
```

- `size` is the length of the record in bytes, with the size field.
- `n` is the header count. A record with the header count `n` has `n + 1`
  fields.
- The offsets count from the start of the record. Field `k` runs from offset
  `k` to offset `k + 1`. The last field runs to the end of the record.
- A string field is `[i32 length][UTF-8 bytes]`. The length -1 is a null
  string.
- A list field is `[i32 count]` and then `count` records. Each record has its
  size first.
- An id field is 16 bytes.

The root record is the profile. Profile field 2 is the list of commands.
Command field 2 is the list of actions.

The walk checks the structure. A failure stops the build with a typed error:

| Check | Error |
|---|---|
| The stream inflates | `Inflate` |
| The stream is complete | `Truncated` |
| No bytes follow the stream | `TrailingBytes` |
| The inflated size is at most the limit | `TooLarge` |
| The size of the root record is the inflated length | `RootSize` |
| Each record and each field is inside its parent | `OutOfBounds` |
| The first offset is the end of the offset table, the offsets do not go down, and the last offset is inside the record | `BadOffsets` |
| The header count is at least the field index that a rule reads | `FieldCount` |
| The header count is at most the limit | `TooManyFields` |
| A string, integer, or id field has the correct length and UTF-8 text | `BadField` |
| A list fills its field exactly | `BadList` |

## Field counts and versions

The tested format has these header counts: profile 89, command 82, action 32
(`TESTED_FIELD_COUNTS`). VoiceAttack 1.16 wrote this format.

- A header count below the largest field index that a rule reads stops the
  walk with `FieldCount`. These indexes are profile 32, command 20, and
  action 30 (`MIN_FIELDS`).
- A header count below the tested count is an older format. The builder
  stops with `OlderFormat`, because nobody checked the meaning of the fields
  in an older format.
- The builder accepts a header count above the tested count, with a format
  note. Each new VoiceAttack version so far added its fields at the end of
  each record.
  An older VoiceAttack profile (version 1.10.3, counts 86, 78, and 30) had
  the fields that the rules read at the same index.
- The version text (profile field 32) must start with digits, a dot, and
  digits. Other text can follow. Else the build stops with `Version`.

## The fields that the rules read

The confidence column gives how sure the meaning is. "High" means that the
field contents and the profile comments agree. "Medium" means that the
meaning fits each use in the tested profile. "Low" means that the meaning is
a guess.

| Record | Field | Meaning | Confidence |
|---|---|---|---|
| Profile | 2 | The list of commands | High |
| Profile | 32 | The VoiceAttack version text | High |
| Command | 0 | The command id (16 bytes) | High |
| Command | 1 | The phrase (VoiceAttack grammar) | High |
| Command | 2 | The list of actions | High |
| Command | 4 | 1 means "the command is on" | Medium |
| Command | 7 | The category | High |
| Command | 15 | The bytes 1, 1 mean "when I say" (a voice trigger) | Medium |
| Command | 19 | The command type: 0 full, 1 prefix, 2 suffix | High |
| Command | 20 | The composite group of a prefix or a suffix | High |
| Action | 1 | The action type | High |
| Action | 3 | The mode of an execute action (by id or by name) | High |
| Action | 5, 6 | Strings. Their meaning depends on the action type. | High |
| Action | 7, 8 | Set-text sources: the variable to copy, the file to read | High |
| Action | 12, 13, 14 | Set-text options: the replace flag, the source kind, the replace text | Medium |
| Action | 16, 17 | Condition actions: the index of the partner action, the nesting level | High |
| Action | 18, 19 | Condition actions: the variable and the operator | High |
| Action | 21, 22 | Condition actions: a compare with a second variable | High |
| Action | 23 | Condition actions: the value kind (1 is text) | High |
| Action | 28 | One byte. 1 probably means "the action is switched off" | Medium (about 80 percent) |
| Action | 30 | Condition actions: a compound condition | High |

For a write to a file (type 44), field 5 is the text and field 6 is the path.
For a set-text action (type 21), field 5 is the variable and field 6 is the
literal value. The source kind (field 13) is 0 for the literal, 1 for a copy
of the variable in field 7, 3 to clear the variable, and 4 for the file in
field 8.

The condition operators of the tested profile are 0, 1, 2, 3, 4, 6, and 9
(`KNOWN_OPERATORS`). For a text compare, 6 is "contains" and 9 is "has not
been set". The rules use only these two.

### Fields that vary and that the builder does not read

| Action type | Field | Values in the tested profile | Meaning |
|---|---|---|---|
| 44 | 10 | 1 on about two thirds of the writes, else 0 | Probably "overwrite" against "append" |
| 44 | 11 | 1 only on writes to other files | Not known. The builder requires 0 on a request write. |
| 19 | 2 | A double, 0.0 on almost all actions, else 1.0 | Not known |
| 19 | 12 | 1 on each `{PREFIX}` block, 0 on each "has not been set" block | A text compare option |
| 20 | 10 | 1 on a few actions | Not known |
| 16 | 10 | Mostly 1 | Probably "wait until the command ends" |
| 37 | 10, 11, 19 | Vary | The builder reads type 37 only in the system-slot parse |

## The action types

The class column tells what an action can do to the request token:

- `write`: the request-file write.
- `sets-variable`: it can change a variable.
- `control-flow`: it can gate or reorder the write.
- `inert`: it cannot change what the command writes.
- `side-effect`: it cannot change the token, but it does something that the
  program does not do.
- `unknown`: nobody knows what it does.

| Type | Meaning | Confidence | Class |
|---|---|---|---|
| 0 | Press keys. Field 4 holds the key codes. | High | side-effect |
| 2 | Pause. Field 2 is the time in seconds (a double). | High | inert |
| 3 | Run an application | High | side-effect |
| 4 | Act on a process by name | Low | side-effect |
| 13 | Say a text with speech synthesis | High | inert |
| 14 | Play a sound file | High | inert |
| 15 | Change the active profile | Medium | side-effect |
| 16 | Execute another command, by id (field 5) or by name (field 6) | High | control-flow |
| 18 | Set a small integer | Medium | sets-variable (not text) |
| 19 | Start a condition | High | control-flow |
| 20 | End a condition | High | control-flow |
| 21 | Set a text variable | High | sets-variable |
| 22 | Run a plugin function | Medium | sets-variable (any) |
| 23 | Write a line to the event log | High | inert |
| 24 | Set the clipboard | Medium | sets-variable (clipboard) |
| 28 | Comment | High | inert |
| 29 | Else | High | control-flow |
| 30 | Start a loop | High | control-flow |
| 31 | End a loop | High | control-flow |
| 34 | Window action by title (focus, show) | Medium | inert |
| 36 | Set a boolean | High | sets-variable (not text) |
| 37 | Set an integer | High | sets-variable (not text) |
| 38 | Set a decimal | Medium | sets-variable (not text) |
| 44 | Write a text to a file | High | write |
| 50 | Switch the listening of VoiceAttack on | Medium | side-effect |
| 51 | Switch the listening of VoiceAttack off | Medium | side-effect |
| 57 | Stop each running command | Medium | control-flow |
| 63 | Else if | High | control-flow |
| 69 | Not known. It has no text field. | Low | unknown |
| 70 | Set an audio level | Medium | side-effect |
| 71 | Run inline C# code | High | sets-variable (any) |
| 74 | Wait for a spoken response. Field 6 gets the result. | Medium | sets-variable |
| 75 | Exit a loop | Medium | control-flow |
| 76 | Reload the voice commands or the profile | Low | side-effect |
| 77 | Set a text variable from an integer | Medium | sets-variable |

`TABLE` in `allow.rs` has these 35 types with a short name. A type that the
table does not list is not allowed before the write.

## The request write and the placeholders

A request write is an action of type 44 whose path (field 6) is exactly
`{TXT:PythonLibPath}\{TXT:RequestFile}` (`REQUEST_PATH`). Its text (field 5)
is the token template of the record.

A token template can have these placeholders. The matcher fills them:

| Placeholder | Source |
|---|---|
| `{CMD}` | The spoken command text |
| `{TXT:x}` with `x` a number variable | The number of the spoken text (see "Numbers") |
| `{TXT:x}` with `x` a constant | The value of the constant (see "Constants") |
| `{TXT:x}` with `x` a phrase slot | The first name of the slot file (see "Slots") |
| `{TXT:SystemName}`, `{TXT:spokenQueuedName}` | A fallback from the system-slot parse or a phrase slot |
| A variable of a `{PREFIX}` rule | The prefix of a composite command (see "Composite commands") |

Any other placeholder gives no token. A record with such a placeholder can
give a miss, never a wrong token. The report counts these records as
`cannot_fire`: the token has another VoiceAttack token (for example
`{INT:x}`), or a text variable that no source above fills.

## The classes of the left-out commands

The builder first applies the classes of the first map extractor (a Python
script), in its order. Then it applies the whitelist to each command that
these classes keep.

| Class | Rule |
|---|---|
| `no_write` | The command writes no request token. |
| `multi_write` | The command writes two or more request tokens. |
| `maintenance` | A full command whose single write is inside a condition block, or that also presses a key (type 0). |
| `orphan_suffix` | A suffix whose composite group has no prefix. |
| `not_understood` | The command has an action, a shape, or a value that the builder does not fully understand. The reason names it. |

These items also leave a command out as `not_understood`:

- The command has no phrase, or the phrase is the name of a category.
- The command type is not 0, 1, or 2.
- A prefix or a suffix has no composite group.
- The command writes an empty text to the request file.
- Command field 4 is not 1, or command field 15 is not 1, 1. Such a command
  can still be the target of an execute action.
- A command setting has an unknown value (see "Format drift").
- A prefix of the composite group has an action that is switched off.

## The whitelist

The builder puts a command into the map only when the whitelist allows each
action at its position. Otherwise it leaves the command out with a reason. The
whitelist is in `allow.rs`.

### Before the write

The builder allows these actions before the write:

1. The inert types 2, 13, 14, 23, 28, and 34. Also type 24 (the clipboard)
   when the token has no `{CLIP}`.
2. The types 18, 36, 37, and 38. They do not change a text variable.
3. Type 21 with the literal source (field 13 = 0) and no file, copy, or
   replace field. A set of a token variable must fit a modelled path (a
   constant or a number variable) at depth 0.
4. Type 21 with another source, only for a variable that is not in the
   token.
5. Condition blocks (19, 29, 63, 20) that hold only the items 1 to 4, and no
   set of a token variable. A block has at most one `else` action. A block
   with two `else` actions has no meaning that the program knows.
6. The system-slot parse (see "Slots").
7. In a suffix, the suffix shape (see "Composite commands").

These rules also apply before the write:

- An action with field 28 = 1 at or before the write leaves the command out.
  The program cannot know what VoiceAttack does with it.
- A set of `PythonLibPath` or `RequestFile` (the variables of the request
  path) leaves the command out.
- A set action (types 18, 21, 36, 37, 38, 74, 77) whose variable name has a
  `{` can set any variable. At or before the write, at any depth, in a
  prefix, or in the suffix shape, it leaves the command out.
- These types before the write leave the command out: 0, 3, 4, 15, 22, 30,
  31, 57, 69, 71, 74, 75, 76, 77, and each type that the table does not
  list.

### The write

- The command has exactly one request write. A second request write is not
  allowed.
- A full command has its write at depth 0, outside a condition block or a
  loop. A suffix can have its write in the known suffix shape only.
- Write field 11 must be 0.
- The request text must not be empty and must have at most 1,024
  characters.

### After the write

The token is already in the request file after the write. Thus the builder
allows these actions after the write:

- The inert types and the side-effect types (0, 3, 4, 15, 50, 51, 70, 76).
- The condition actions 19, 20, 29, and 63.
- The set actions 18, 21, 24, 36, 37, 38, and 77.
- Type 16 (execute a command) with a target that passes the checks of
  "Execute targets".

These actions after the write leave the command out: a file write (44), the
types 22, 69, 71, and 74, the loop types 30, 31, and 75, and type 57.

An action with field 28 = 1 after the write counts as present for these
rules.

### Execute targets

The builder allows a type 16 action after the write only when its target is
sure and cannot write a request token.

- The mode (field 3) and the target must agree. "By name" needs the zero id
  and one command with the exact name. "By id" needs one command with the
  id. A name next to the id must name the same command.
- Execute by name. The builder finds the target only when no other phrase
  can give the same name. The check reads the groups, the ranges, the lists
  with `;`, and the placeholders of each phrase. A placeholder can give any
  text. A phrase that the check cannot read gives each name. The names
  compare in one normal form: lowercase, and each character that is not a
  letter or a digit is a space. A target name must have single spaces.
- The target must write no request token, also through the commands that it
  executes. The builder keeps the result of each target for the whole build.
  Commands that execute each other in a loop are one group (the strongly
  connected components of Tarjan). More than 32 steps count as a target
  that can write.
- A target must not have a plugin action (22).
- A target can have inline code (71) only when the SHA-256 of its code is in
  the table `REVIEWED_CODE` of `allow.rs`. The hash input is the fields 5
  (the code), 6 (the assemblies), and 11, each with its length first. A
  person reads the full code before its hash goes into the table. Inline
  code directly in a command of the map is never allowed.
- A target can write another file only when the path text after the last
  placeholder is exactly `\` and one of these names (ASCII case does not
  matter): `EDCoPilot.VApath.txt`, `LaunchEDCoPilot.bat`,
  `EDCoPilot.VAPversion.txt`. Each other path can reach the request file.
  Windows also opens a file through `.`, `..`, short 8.3 names, a point or a
  space at the end, and streams. Add a name to `OTHER_FILES` only after a
  check of the commands that write it.
- A target with an unknown command setting counts as a target that the
  builder does not know.

### Slots

A phrase slot is a placeholder of the phrase that the program fills from a
vocabulary file in `Dynamic var files`: `{TXT:System<n>}`, `{TXT:Group<n>}`,
`{TXT:Voice<n>}`, and `{TXT:POIType<n>}`.

- A token that takes the text of a phrase slot directly stays only for a
  slot kind whose file holds one plain name: group, voice, and point of
  interest. VoiceAttack writes the whole text of the slot variable. The
  matcher writes the first name. These are the same text only while the
  file holds one name. The form check of the slot files (`dynamic_vocab`)
  uses the same rule.
- A system slot file holds `[name;alternate;...]` or a "Not set" text. It
  can hold a list. Thus a token that takes a system slot directly leaves the
  command out.
- A variable of no slot kind in the token leaves the command out.
- The system-slot parse is one fixed sequence of ten actions (`SLOT_SHAPE`).
  It reads the slot file `{TXT:DynamicVarLib}\EDCoPilotSystem<n>.txt` into
  `SystemFromFile` and takes the text between `[` and the first `;` into
  `SystemName`, with the integer variables `Startpos` and `SysLength`. The
  file number must be the number of the phrase slot. The parse counts only
  at depth 0, with the tested names of its variables. Another set of one of
  its variables before the write leaves the command out.
- The parse checks the fields 10, 11, and 19 of its four set-integer actions
  with the tested values: (0, 0, 0), (1, 1, 0), (0, 0, 0), and (0, 1, 1).
  The second action has no value (field 15).
- The output of the parse (`SystemName`) is not a phrase slot.

### Numbers

A number command takes a number from the spoken text.

- A number variable `x` has exactly one plain set-text with the value
  `{TXTNUM:LastCMD}`, at depth 0, before the write. Each set of `LastCMD` is
  `{CMD}`, and each comes before it.
- VoiceAttack makes the text of `{TXTNUM:x}` from the whole command text. It
  keeps the digits, the `.`, and the `-`. The matcher keeps the ASCII digits
  in phrase order. Thus a command whose phrase has a `.`, a `-`, or another
  numeral (for example a superscript digit) outside its number ranges is
  left out. ASCII digits outside the ranges pass, because both give the
  same text. Text in placeholders does not count.
- One exception: a plain alternative with a `-` passes when its group has a
  twin. The twin is a plain alternative of the same group at the same level,
  with no `-`. Both must have an ASCII letter. Both must be equal when you
  remove each `-` or change it to one space, without regard to case and to
  the outer spaces. An alternative with a `-` and a digit gets no exception.
- The check runs in three places. The builder checks the literal phrase and
  the phrase with the grammar of each profile variable.
  `resolve_runtime_grammar` checks the phrase with the runtime grammar of
  `EDCoPilot.VoiceCommands.ini`, also when the path of that file is empty.
  It gives each removed command back with the reason. The load puts it into
  the left-out list (`not_understood`).
- A suffix with a number variable is left out. VoiceAttack makes its number
  from the prefix text too.
- The matcher gives no number token when the text of a slot has a digit, a
  `.`, or a `-`.

The check is `core/src/number_phrase.rs`.

### Composite commands

A composite command is a prefix (command type 1) and a suffix (command type
2) of the same composite group, spoken together.

- A prefix writes no request token. It has one or more plain set-text
  actions and no other set. Each set must have a value.
- A suffix must have the known shape. First come `{PREFIX}` blocks: a
  condition with a text compare "contains" on the prefix text, no second
  variable, no compound condition, field 12 = 1, field 2 = 0.0, and one
  plain set-text in the block. Then one "has not been set" block (operator
  9, no compare value) with inert actions, an `else` branch with only the
  write, and the end.
- The variable of the "has not been set" block must be a rule variable that
  is in the token.
- Variable names that differ only in case leave the prefix or the suffix
  out.
- A suffix that sets a variable of its prefix rules outside the `{PREFIX}`
  blocks is left out.
- A prefix with an action that is switched off leaves its whole composite
  group out.

### Constants

A constant is a variable of the token with a fixed value. The only constant
name is `SystemName` (`CONST_VARS`).

- The value comes only from the set-text actions before the write. The
  names compare without regard to case.
- Exactly one such set must exist outside the system-slot parse and the
  suffix shape. It must be plain and at depth 0. Its value must not be
  empty and must not have a `{`.
- The record takes the value from that action. A second set, a set in a
  condition, or a value with a token leaves the command out. A set after
  the write does not change the constant.

### Profile variables

A profile variable is a phrase variable whose grammar comes from the
profile. One set-text action of the profile gives the variable a grammar
text. The load puts this grammar into each phrase that uses the variable.

- The variable has exactly one set-text in the whole profile (names without
  regard to case). The set has the literal source and a value without a
  placeholder.
- The set must be on. A variable whose only set has field 28 = 1 is not a
  profile variable.
- A variable that a set-text with another source sets is not known.
- A variable that the text of a code or plugin action names is not known.
  The builder searches the code texts only up to 1 MiB. Above this size,
  each name counts as named by code.
- A variable whose name fits a name pattern (see below) is not a profile
  variable.
- The builder checks at most 64 variables in one build. A further variable
  is not a profile variable (a miss).

The only profile variable of the tested profile has its set inside a
condition. The program uses its grammar and assumes that the condition is
true when EDCoPilot runs. Open question 7 asks for a live check.

Name patterns of sets with a token in the variable name:

1. Each set with a token in the variable name gives a name pattern. The
   literal text stays. Each token gives any text, with one exception.
2. The exception: a `{TXT:V}` token gives the text of an integer when each
   writer of `V` in the profile is a type 77 action. The text of an integer
   is an optional minus and one or more digits. The pattern also accepts
   `Not set`, the text of a variable without a value.
3. A writer of `V` is a set action with the name `V` (types 18, 21, 36, 37,
   38, 74, 77), or a code or plugin action whose text has the name `V`. A
   set with a token is a writer of each variable that its pattern fits.
4. A name that is not a string, a `{` without a `}`, or a token with a `{`
   in it gives any text for the rest of the name.
5. The compare does not look at case.

An example with invented names: the set `Lamp{TXT:Counter}`, where only type
77 writes `Counter`, can write `Lamp12` and `Lamp-3`. It cannot write
`LampWords`. When `Counter` has another writer, the set can write each
variable whose name starts with `Lamp`.

### The variables of the request path

A writer of `PythonLibPath` or `RequestFile` that is not a plain set-text
makes the request path unknown. Examples are a type 74 or type 77 action with
the name in field 6, or a set with a token whose name pattern fits the
variable. When such a writer is anywhere in the profile, each command with a
request write leaves the map. Code and plugin actions are not in this rule:
reviewed code sets a path variable in a command that runs after the write.

### Blocking phrases

The builder gives a blocking phrase (`BuiltMap::blocking`) for each left-out
command that VoiceAttack recognises by voice. The matcher keeps each such
phrase as a path without a token. Thus a text that names a left-out command
cannot become another command through a sound-alike variant, a synonym, a
filler drop, or a fuzzy word.

1. A left-out command gives a blocking phrase when field 4 is 1 (on), field
   15 is 1, 1 (a voice trigger), the phrase is not empty and not in `((` and
   `))` (an internal command), the command type is 0, 1, or 2, and a prefix
   or a suffix has a composite group. When the builder cannot read field 4
   or field 15, the command counts as spoken.
2. The classes `no_write`, `multi_write`, `maintenance`, and
   `not_understood` can hold such a command. The class `orphan_suffix`
   cannot. VoiceAttack recognises a suffix only after a prefix of its group.
3. A command that the number check of the load takes out also gives a
   blocking phrase (`removed_as_blocking`).
4. A phrase longer than 4,096 characters gives a blocking phrase from its
   first 4,096 characters, only when each group and each variable closes in
   them.
5. The load puts the runtime grammar and the profile variables into each
   blocking phrase (`resolve_blocking`). The load skips and counts a phrase
   that still has a `{TXT:EDCP_` variable, or that does not parse.
6. `Matcher::with_blocking` keeps each phrase as a path without a token. A
   full command gives a grammar. A left-out suffix joins its composite group
   as a suffix without a token. A left-out prefix joins its group, and each
   composite with it gives no token. A composite group of the working
   commands stays also when it has only prefixes or only suffixes.
7. A path without a token blocks also when it fails its required words. It
   blocks when its confidence is not below the path that the matcher would
   send. The margin is the tie margin of `decide`.
8. The records do not change. The command list and the catalog do not show
   the blocking phrases. The left-out list has them.
9. `shadowed_phrases_with` compares the working texts with the texts of the
   full blocking phrases and of the composites with a left-out part. A group
   with more than 400 texts on one side is not compared. The warning text
   is: `... is also a phrase of a command that the program cannot send, the
   program does not send a command for it`.

## Format drift

A new VoiceAttack version can move a field or change its encoding. These
guards find such a change.

### Length signatures

Each field in these tables has one length in each record of its kind in the
tested profile (`PROFILE_SIGNATURE`, `COMMAND_SIGNATURE`,
`ACTION_SIGNATURE` in `record.rs`).

| Record | Field: length in bytes |
|---|---|
| Profile | 0: 16. 3, 5, 11, 12, 14, 15, 21, 22, 23, 25: 1. 6, 16: 2 |
| Command | 0: 16. 3, 4, 8, 16: 1. 14, 15: 2. 9 to 13, 17, 18, 19: 4 |
| Action | 0: 16. 28: 1. 1, 9 to 13, 16, 17, 19, 20, 21, 23, 29: 4. 2, 3: 8. 26, 27: 12. 24, 25: 16 |

- Another length in a profile field or an action field stops the build with
  `Layout`.
- Another length in command field 0 (the id) or command field 19 (the
  command type) stops the build. Each command needs them.
- Another length in another command field is a setting of that command. The
  builder leaves the command out.
- A field that is a null string in each record is not in the tables.
  Another profile can give it a text (command field 5).
- The profile has one record. Thus its table holds only the fields of 1 or
  2 bytes, which cannot be a string.

### Tested settings

- `TESTED_SETTINGS` lists, for each modelled action type (21, 44, 19, 20,
  29, 63, 16, and 37 in the slot parse), the fields that the builder does
  not read and that have one value in each such action of the tested
  profile. A modelled action with another value leaves its command out. The
  tested value of each field is `tested_default` in `record.rs`.
- A null string is equal to an empty string in the string fields 5, 6, 7,
  8, 14, 15, 18, and 22. A null list is equal to an empty list in field 30.
  Field 9 is an integer (-1 or 0), not a string.
- `COMMAND_SETTINGS` lists 62 command fields with one value in each command
  of the tested profile. A command with another value leaves the map with a
  reason that names the field. A null string and an empty string are equal
  in the command fields 5, 52, 61, 62, 80, and 82.
- A newer format adds fields at the end of each record. An appended field
  of a modelled action with another value than the most common value of
  that field for the type leaves the command out.

### Thresholds

A change that many records share is a change of the format, not a setting
of the author. These rules stop the build:

| Rule | Limit | Error |
|---|---|---|
| A field of `SHARED_FIELDS` (action fields 25, 26, 27, 29, 31, 32) has another value, over all actions | 50 actions and more than 5 percent | `FormatChanged` |
| The same, over the actions of one type | 50 actions and more than 5 percent of that type | `FormatChanged` |
| A tested field of a modelled action type has another value | 50 actions and more than 20 percent of that type | `FormatChanged` |
| A field of `COMMAND_SETTINGS` has another value | 50 commands and more than 5 percent | `FormatChanged` |
| One reason of a format kind leaves out commands with a request write | 50 commands and more than 25 percent | `ReasonShare` |

The format kinds of reasons are: an action setting that the program does not
know, a command setting that the program does not know, and an action type
that the table does not list, before the write. The reason "the command sets
a part of its token in a way that the program does not model" is not of a
format kind.

`FormatChanged` names the record kind, the field, and the action type when
the count is of one type. The values can differ.

### Totals

These checks at the end of the build stop it with `Totals`:

| Check | Meaning |
|---|---|
| `Writes` | The writes in the records and in the left-out commands do not add up to the writes in the profile. This is a check of the program. The text tells the user to tell the developer. |
| `TooFewRecords` | The map has fewer than 200 records. The error names the largest group of left-out commands and its count. |
| `NoFixedRecord` | No record writes a fixed token. |
| `NoRequestWrite` | No command of the profile writes a request. |

### Which error stops the whole build

| Error | One odd command can cause it | Why the error stops the build |
|---|---|---|
| `Inflate`, `Truncated`, `TrailingBytes`, `TooLarge` | No | The data is not one complete stream. |
| `RootSize`, `OutOfBounds`, `BadOffsets`, `BadList` | Yes | The record bounds are not sure. The program cannot read the rest of the file safely. |
| `FieldCount`, `OlderFormat` | Yes | The fields of an older format have meanings that the program does not know. |
| `TooManyFields`, `TooMany` | Yes | The limits keep the work and the memory small. |
| `Layout` and `BadField` of an action | Yes | The rules of the whole profile read each action: the writers of the request path, the profile variables, and the execute targets. |
| `Layout` of command field 0 or 19, `BadField` of a command | Yes | The execute targets and the phrase need these fields. |
| `Version` | No | It is a field of the profile record. |
| `Totals` | No | They need many commands. |

The report also counts these items. The saved report keeps the counts. A
saved report without them reads as 0, so the schema stays 1.

- The actions with an unknown setting, by action type and field.
- The records with a phrase variable that is no slot, no profile variable,
  and no `EDCP_` variable (`unknown_phrase_vars`). Such a record never
  matches. `check` then shows the line `<n> commands have a phrase variable
  that the program does not know`.
- The records that cannot fire (`cannot_fire`).

## Bounds

Each bound gives the strict result: a miss or a typed error, never a wider
match.

| Item | Bound |
|---|---|
| Inflated profile | 64 MiB (`MAX_INFLATED`) |
| Header count of a record | 1,024 |
| Commands, actions, actions in one command | 20,000, 200,000, 2,000 |
| Phrase | 4,096 characters. A longer phrase leaves its command out. |
| Request text | 1,024 characters |
| Depth of groups in a phrase | 64 levels (`MAX_GROUP_DEPTH`). A deeper phrase can give each name, fails the number check, and never matches. |
| Steps of an execute chain | 32 |
| Distinct names of execute by name in one build | 64, each with at most 64 words. A further name is a target that the program cannot find. |
| Checked profile variables | 64 |
| Text of code and plugin actions that the name search reads | 1 MiB |
| Name patterns | 32 distinct names, a name of at most 256 bytes, at most 64 parts. Beyond the bound, one pattern fits each name. |
| Integer variables of the name patterns | 16, with at most 4 rounds of the loop. Beyond the bound, each token gives any text. |
| Action types in the setting tally | 256. The other types count together. |
| Setting counts in the report | 256 |
| Runtime grammar file | 4 MiB. A larger file reads as a file that does not read. |
| Texts of one composite group in the shadow check | 400 |

The search of the placeholders is linear in the length of the text. The
name check of an execute by name works on the set of name positions one time
for each alternative, so groups in groups do not give exponential work. The
match of a name pattern keeps the set of positions that it can reach, so it
does not backtrack.

## Test strategy

The repository has no EDCoPilot data. Thus the normal tests use invented data
only. The live tests use the EDCoPilot installation of the developer and
private files that git does not track.

### Normal tests (`cargo test --workspace`)

- `core/src/profile/build_tests.rs`: one test or more for each rule. Each
  test builds a small synthetic profile with `test_support.rs` and checks
  the record or the reason.
- `core/src/profile/bound_tests.rs`: crafted profiles at the limits. Each
  build must end in 10 seconds (debug build).
- `core/src/profile/fixture.rs`: an invented profile with an invented
  harbour and lighthouse vocabulary in the real format. Only the name
  patterns that the code must match are real (for example `SystemName`,
  `LastCMD`, `System<n>`, the `EDCP_` prefix, and the request path).
  `FIXTURE_COUNTS` pins its counts.
- `core/tests/fixture_golden.rs`: compares each record, each left-out
  command, each profile variable, the report counts, and each blocking
  phrase of the fixture build with `core/tests/golden/fixture_map.txt`.
  When a change of the builder is correct, write the file again with
  `VOICEBRIDGE_GOLDEN_WRITE=1` and read the difference in git before the
  commit.
- The tests of `app` and `ui` write the fixture to a temporary EDCoPilot
  folder (`write_fixture_edcopilot`).

### Live tests (ignored)

The live tests read the EDCoPilot folder and write nothing there. Run them
by name only. Some other ignored tests of the workspace download a model or
open the microphone.

| Test | What it checks | Environment |
|---|---|---|
| `core/tests/profile_live.rs` | The build of the real profile works, and its counts are in a band around the last recorded numbers. | `VOICEBRIDGE_EDCOPILOT_DIR` |
| The snapshot test of `profile_live.rs` | The build is equal to a private snapshot. | `VOICEBRIDGE_MAP_SNAPSHOT`. With `VOICEBRIDGE_MAP_SNAPSHOT_WRITE=1`, it writes the snapshot. |
| `core/tests/live_probes.rs` | The real phrases of the private probe file give the expected tokens. | `VOICEBRIDGE_LIVE_PROBES`, default `private/live_probes.tsv` |
| The shadow test of `live_probes.rs` | Counts the working texts that are also texts of a left-out command. | `VOICEBRIDGE_SHADOW_OUT` writes the texts to a private file. |
| `app/tests/live_probes.rs` | The `@app` sections of the probe file through the console program (`text`, `commands`, `check`, never `--send`). | Same probe file |
| The live test of `app/tests/cli.rs` | The load and the console program on the real folder. | `VOICEBRIDGE_EDCOPILOT_DIR` |

The snapshot and the probe file hold EDCoPilot text. Keep them in `private/`
in the repository folder, which `.gitignore` names. The snapshot write
refuses a path in the repository that is not under `private/`. A relative
path starts at the repository folder.

Example:

```text
VOICEBRIDGE_EDCOPILOT_DIR=C:\EDCoPilot cargo test -p edcopilot-voicebridge-core --test profile_live -- --ignored --nocapture
```

`docs/DEVELOPMENT.md` lists all the environment variables of the tests.

### After an update of EDCoPilot or VoiceAttack

1. Run `core/tests/profile_live.rs`.
2. If the build stops, read the typed error. Compare the field counts, the
   signatures, and the settings with this document.
3. If the counts are outside the bands, compare the left-out reasons with
   "The last recorded numbers".
4. Run the probes. Update the private probe file when EDCoPilot changed a
   phrase or a token.
5. Write a new private snapshot when the change is correct.
6. Update this document and the bands of `profile_live.rs`.

## Open questions

Each question needs a capture of the request file while VoiceAttack and
EDCoPilot run. Do these experiments:

1. Field 28. Speak one of the commands that have a switched-off request
   write and one switched-on request write. Record the token. If the token
   is the one of the switched-on write, 1 means "switched off". Then these
   commands can go into the map.
2. Write flags. Find out if fields 10 and 11 of type 44 mean "overwrite"
   against "append". This does not change the token, but it can change what
   EDCoPilot reads.
3. Types 69 and 76. Find out what these actions do. Neither has a text
   field.
4. Composite tokens. Speak a prefix and a suffix of one composite group.
   Compare the captured token with the token of the program. Nobody has
   captured a composite token yet.
5. `{CMD}`. Speak a command whose token has `{CMD}`. Compare the captured
   text with the text of the program.
6. Group slots. The real installation names the group files
   `EDCoPilotBookmarkGrp<n>.txt`, and the phrases use `{TXT:Group<n>}`. The
   program does not load these files under the slot names, so the group
   commands do not match. Capture the token form of a group command before
   a change of the mapping.
7. The profile variable. Its only set is inside a condition. Speak a phrase
   that uses it while EDCoPilot runs, and confirm that VoiceAttack
   recognises it.
8. Strictness against new profiles. Each new VoiceAttack version that adds
   a field gives a format note, and an older format stops the build.
   Collect the header counts of profiles of other versions. Then decide if
   the builder can accept more formats.
9. Slot files of other installations. The rules about the slot file format
   come from one installation. Collect the forms of the files of other
   users.

## The last recorded numbers

These counts come from the profile of EDCoPilot 1.11.755 (VoiceAttack 1.16),
in a debug build. The bands of `core/tests/profile_live.rs` come from them.

| Item | Count |
|---|---|
| Size of the `.vap` file | 994,067 bytes |
| Inflated size | 9,298,154 bytes |
| Commands, actions, most actions in one command | 3,058, 19,569, 103 |
| Action types in the profile | 35 |
| Commands with a request write | 3,002 |
| Records that the builder makes | 2,486 |
| Records after the load | 2,485 |
| Left out at the build, after the load | 572, 573 |
| `not_understood` at the build, after the load | 502, 503 |
| `no_write`, `multi_write`, `maintenance`, `orphan_suffix` | 45, 12, 5, 1 |
| Profile variables | 1 |
| Blocking phrases of the build | 564 |
| Blocking phrases in the matcher | 558 |
| Blocking phrases skipped at the load (a `{TXT:EDCP_` variable without a value) | 7 |
| Records that cannot fire | 19 |
| Records with an unknown phrase variable | 0 |
| Working commands with a shadowed text | 1 |
| Private probes: core, app | 300, 23 |

The reasons of the 502 `not_understood` commands at the build:

| Reason | Commands |
|---|---|
| The command sets a part of its token in a way that the program does not model | 482 |
| Type 16 after the write: the program cannot find the command that it runs | 8 |
| An action before the write is switched off in the profile | 7 |
| Type 69 before the write | 2 |
| Type 74 before the write | 1 |
| The slot file number is not the phrase slot number | 1 |
| A point, a minus sign, or another numeral outside the number ranges | 1 |

The margins of the format rules on this profile:

| Rule | Largest count | Limit |
|---|---|---|
| A tested field of one type | 8 of 3,052 actions of type 44, field 11 | 50 actions and 20 percent |
| A shared field, all actions or one type | 0 actions | 50 actions and 5 percent |
| A command setting | 0 commands | 50 commands and 5 percent |
| A reason of a format kind | 0 of 3,002 commands | 50 commands and 25 percent |
| Checked profile variables | 1 | 64 |
| Distinct names of execute by name | 17 | 64 |
| Sets with a token in the name | 4 | 32 |
| Text of code and plugin actions | 16,805 bytes | 1 MiB |
| Longest phrase, longest request text | 434, 205 characters | 4,096, 1,024 |
| Commands, actions, actions in one command | 3,058, 19,569, 103 | 20,000, 200,000, 2,000 |
| Action types | 35 | 256 |
| Runtime grammar file | 64,683 bytes | 4 MiB |

Times: a release build needs about 13 ms for the full profile. A debug build
needs about 150 ms. The inflate step alone needs about 4 ms (release).
