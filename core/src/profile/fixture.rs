//! The synthetic profile for tests: an invented harbour and lighthouse
//! vocabulary in the format of an EDCoPilot profile.
//!
//! The repository holds no EDCoPilot content. The tests of the workspace
//! use this profile in place of a real map. Each phrase, token, grammar
//! line, category, harbour name, and person name here is invented. Only
//! the NAME patterns that the code must match are real: `SystemName`,
//! `LastCMD`, `System<n>`, `Group<n>`, `EDCoPilotSystem<n>.txt`,
//! `EDCoPilotBookmarkGrp<n>.txt`, the `EDCP_` prefix,
//! `{TXT:DynamicVarLib}`, and the request path.
//!
//! The group files have the names of the real installation:
//! `EDCoPilotBookmarkGrp<n>.txt`, while the phrases use `{TXT:Group<n>}`.
//! The program loads a file under its name without `EDCoPilot`, so the
//! slots `Group<n>` get no names, and the group commands do not match.
//! This is the behaviour of the program on a real installation today. The
//! mapping needs a live capture of the token form first.
//!
//! The token shapes imitate the real shapes, because the matcher and the
//! writer parse them: colons, `=`, quoted values, `{TXT:Var}` placeholders,
//! `{CMD}`, and one token that ends with CR LF.
//!
//! The builder needs at least [`super::MIN_RECORDS`] records. Thus the
//! profile has more than 200 commands: most of them come from small
//! families of the same shape (flags, lamps, rooms, crew stations, days,
//! compass points).
//!
//! # The properties and their commands
//!
//! | Property | Invented phrase | Invented token |
//! |---|---|---|
//! | Fixed, many words | `help tide log` | `Guide:Panel:TideLog` |
//! | Fixed, one word (exact-match rule) | `belay` | `Belay` |
//! | Two phrases that differ by one word | `raise the blue flag`, `raise the red flag` | `Flag:Blue:Up`, `Flag:Red:Up` |
//! | One phrase is a prefix of another | `ring bell`, `ring bell twice` | `Bell:Ring`, `Bell:RingTwice` |
//! | Alternatives and optional parts | `[raise; hoist] the storm [cone; signal] [now;]` | `Signal:StormCone` |
//! | Several phrases in one command | `toll the chapel bell; strike the chapel bell` | `Bell:Chapel` |
//! | A token that ends with CR LF | `sound the noon gun` | `Gun:Noon\r\n` |
//! | Two commands with one phrase (a tie) | `close the shutter`, `[close; shut] the shutter` | `Shutter:Close`, `LampShutter:Close` |
//! | Number, range 0 to 100 | `lamp brightness [0..100] [percent;]` | `Prefs:LampBrightness="{TXT:Level}"` |
//! | Number, scaled range (INI) | `{TXT:EDCP_HORNVOLUME}` = `[fog; mist] horn volume [1..20, 5] [percent;]` | `Prefs:HornVolume="{TXT:Volume}"` |
//! | Number, thousands | `show buoys within [1..50] thousand yards` | `ShowBuoysWithin={TXT:Yards}000` |
//! | Number word at the end | `hoist pennant [1..12]`, `remove crate [1..30]` | `Pennant:{TXT:PennantNo}` |
//! | Number in the middle | `ring the bell [1..12] times`, `[nearest berth;] [size;] [1..4;] [old; new; spare;] steam crane` (only the last two words are required, as the size) | `Bell:Times={TXT:Rings}`, `Berth:Steam Crane~{CMD}` |
//! | Number next to `for`, `to`, `by`, `a` | `net reminder [0..99]` (INI), `set crate count to [1..99]`, `add lamp oil to the store by [1..50] barrels`, `move the ferry to mooring [1..50]` | `SetNetReminder:{TXT:NetCount}` |
//! | Two numbers, one value | `pick mooring [1..9] [0..9]`, `set the context to mooring [1..9] [0..9]` | `PickMooringVoice:{TXT:MooringNo}` |
//! | Two numbers after the first words of a harbour command (a synonym value) | `sail for buoy [1..9] [0..9]` | `SailForBuoy:{TXT:BuoyNo}` |
//! | Optional leading zero (INI) | `{TXT:EDCP_MOORING}` = `[sail to mooring; mooring] [0;] [1..50]` | `SelectMooring:{TXT:MooringNo}` |
//! | Minus sign outside a range (INI), left out at the load | `{TXT:EDCP_WICKLAMP}` = `wick-lamp level [1..9]` | `WickLamp:{TXT:WickNo}` |
//! | System slot with the slot parse | `{TXT:EDCP_SAILTO} {TXT:System<n>}`, n = 1 to 12 | `HarbourAt:{TXT:SystemName}:sail to` |
//! | Profile variable (one grammar in the profile) | `{TXT:HarbourQuery} {TXT:System<n>}` | `HarbourInfo:{TXT:SystemName}:{CMD}` |
//! | Group slot (its file name does not give the slot, no match) | `hail {TXT:Group<n>}`, n = 1 to 4 | `Hail:{TXT:Group<n>}` |
//! | Constant with a fixed slot | `{TXT:EDCP_SAILTO} {TXT:EDCP_OLDMILL}` (`SystemName` = `Dunmore`) | `HarbourAt:{TXT:SystemName}:sail to` |
//! | Constant, name absent from the INI | `{TXT:EDCP_SAILTO} {TXT:EDCP_LOSTKEEPER}` | as above |
//! | Constant, name with a range (no forms) | `{TXT:EDCP_SAILTO} {TXT:EDCP_WIDEPEAK}` | as above |
//! | Constant that is also a slot name | `{TXT:EDCP_SAILTO} {TXT:EDCP_ROSEQUAY}` (`Ollin`, also in `System5`) | as above |
//! | Constant whose system has digits | `{TXT:EDCP_SAILTO} {TXT:EDCP_NORTHMOLE}` (`Skerry 409`) | as above |
//! | Constant whose system is equal to its name | `{TXT:EDCP_SAILTO} {TXT:EDCP_KESTON}` (`Keston`) | as above |
//! | Profile variable and a constant | `{TXT:HarbourQuery} {TXT:EDCP_ROSEQUAY}`, `{TXT:HarbourQuery} {TXT:EDCP_NORTHMOLE}` | `HarbourInfo:{TXT:SystemName}:{CMD}` |
//! | Slot command with a sound-alike word (`course`) | `lay course to {TXT:System11}` | `PlotCourse:{TXT:SystemName}` |
//! | Composite prefix (sets a mode) | `[give me the;] [dearest; cheapest] [sell; sale; buy; purchase] bid` | sets `offer_scope` = `Top:` |
//! | Composite prefix without a buy or sell word | `cargo report on` | sets `offer_scope` = `Report:` |
//! | Composite suffix (names an item) | `[rate; offer;] [for;] cork` | `{TXT:TradeSide}{TXT:offer_scope}Cork` |
//! | Composite suffix, runtime grammar | `{TXT:EDCP_RATE_ROPE}` | `{TXT:TradeSide}{TXT:offer_scope}Rope` |
//! | Full command equal to a prefix | `sell at dock; dock sell` | `FindDockSell` |
//! | One required word, optional words | `[find; seek;] cork` | `SetCargo:Cork` |
//! | Runtime grammar (INI list template) | `{TXT:EDCP_SEEK_ROPE}` | `SetCargo:Rope` |
//! | Runtime grammar, several phrases | `{TXT:EDCP_HOMEPORT}` = `row home; sail for home port; take [me;us] back` | `SailHomePort` |
//! | Unresolved grammar (absent from the INI) | `{TXT:EDCP_GATECHIME}` | `Gate:Chime` |
//! | Needs a value that the phrase does not give | `set lamp colour`, `lock the boathouse` | `Lamp:Colour={TXT:LampColour}` |
//! | Working phrase that a value command also has | `[paint the lamp; set lamp colour]` | `Lamp:Paint` |
//! | Execute after the write, harmless target | `sound the horn` (runs `horn blast effect` by id) | `Horn:Blast` |
//! | Sound-alike: `lock` -> `log` | `open tide log` (INI), `help tide log`; `lock current ledger`, `swing the lock gates`, `log the wind speed` as heard; `lock lantern` needs a value, and its variant is the phrase `log lantern` | `OpenTideLogPanel`, `LanternLog:Open` |
//! | Sound-alike: `by`, `cell`, `higher`, `sail` | the composite prefixes, `highest water mark`; `sort nets by length`, `sort stored crates by size`, `lamp cell spares`, `higher tide alerts` as heard | `BuyTop:Cork` |
//! | Sound-alike: `free` -> `three`, `two` -> `to` | `net reminder [0..99]`; `pilot cutter mark two` as heard | `SetNetReminder:3` |
//! | Sound-alike: `kurs` -> `course` | `lay course for home port` | `PlotCourse:HomePort` |
//! | A phrase with a synonym key (`steer`) | `steer the pilot boat` | `PilotBoat:Steer` |
//! | Filler words that the phrase needs | `head for the lighthouse`; names `The Old Mill`, `Tam The Ferryman` | `HeadForLighthouse` |
//! | Cut-off sentence probes | `lamp off`, `belay`, `dump the nets` | `Lamp:All:Off` |
//! | Left out: no write | `load harbour queries` (sets the profile variable), `lock the wind speed` (its sound-alike variant is a working phrase) | none |
//! | Left out: two writes | `ring all bells` | `Bell:One`, `Bell:Two` |
//! | Left out: orphan suffix | `[for;] driftwood` (group `lost cargo`) | `{TXT:TradeSide}Driftwood` |
//! | Left out: maintenance | `reset the lamp profile` (key press), `rebuild the lamp profile` (write in a condition) | `LampProfile:Reset` |
//! | Left out: unknown action type | `polish the lens` (type 99) | `Lens:Polish` |
//! | Left out: unmodelled token part | `report the wind` | `Wind:{TXT:Wind}` |
//! | Left out: code in a target | `test the lamp` (runs `lamp self check`), one fuzzy word from the working `test the lamps` | `Lamp:Test` |
//!
//! The categories are in [`CATEGORIES`]. The vocabulary files are in
//! [`VOCABULARY`], the INI text is [`VOICE_COMMANDS_INI`].

use std::path::Path;
use std::sync::OnceLock;

use super::build_map;
use super::test_support::{
    command_fields, condition, deflate, end_condition, exec_by_id, inline_code, key_press,
    null_str, plain_action, profile, record, say, set_text, set_text_copy, slot_parse, sound,
    str_field, write,
};
use crate::command_map::Command;

/// The category names of the fixture.
pub const CATEGORIES: [&str; 8] = [
    "Lamp room",
    "Flags and signals",
    "Bells and horns",
    "Harbour traffic",
    "Cargo market",
    "Hold and stores",
    "Tide and weather",
    "Logbook",
];

/// The composite group of the cargo market.
pub const MARKET_GROUP: &str = "trade lookup";

/// The number of system slots (`System1` to `System12`).
pub const SYSTEM_SLOTS: u32 = 12;

/// The number of group slots (`Group1` to `Group4`).
pub const GROUP_SLOTS: u32 = 4;

/// The grammar of the profile variable `HarbourQuery`.
pub const HARBOUR_QUERY: &str =
    "[tide at; depth at; berths in; moorings in; weather at; pilots in]";

/// The text of `working\EDCoPilot.VoiceCommands.ini`.
pub const VOICE_COMMANDS_INI: &str = "\u{feff}\\\\ Voice commands file of the invented harbour profile. Test data only.\r
\\\\ Each line is KEY = description|grammar.\r
\r
[Harbour]\r
SAILTO = Sail to <harbour> (first part)|sail to; steer to\r
HOMEPORT = Sail to the home port|row home; sail for home port; take [me;us] back\r
MOORING = Sail to a numbered mooring (1-50)|[sail to mooring; mooring] [0;] [1..50]\r
WICKLAMP = Level of the wick lamp|wick-lamp level [1..9]\r
NETREMINDER = Reminder level for the nets|net reminder [0..99]; net reminder [10..140, 10]\r
HORNVOLUME = Fog horn volume (5 to 100 in steps of 5)|[fog; mist] horn volume [1..20, 5] [percent;]\r
TIDELOG = Tide log|[open; show;] [the;] [tide log; harbour log] [panel;]\r
\r
[Keepers-for-sail-to-commands]\r
OLDMILL = The Old Mill|The Old Mill\r
TAMFERRY = Tam The Ferryman|Tam The Ferryman\r
ROSEQUAY = Rose Quay|Rose [Quay;]\r
HALVARD = Halvard Crane|Halvard Crane|Halvard\r
KESTON = Keston|Keston\r
NORTHMOLE = The North Mole|North Mole\r
WIDEPEAK = A name with a range|peak [1..3]\r
\r
[<Cargo>]\r
ROPE = Rope|rope\r
SALT = Salt|[salt; rock salt]\r
TIMBER = Timber|timber\r
LAMPOIL = Lamp oil|lamp oil\r
\r
[Market - Cargo]\r
SEEK_<Cargo> = Search panel for this cargo|[find; seek;] <Cargo>\r
RATE_<Cargo> = Suffix part of the cargo rate search|[rate; offer;] [for;] <Cargo>\r
";

/// The vocabulary files: file name and content. The first entry of a slot
/// is its name. `System2` and `System3` sound alike. `System7` is an
/// English word. `System11` and the third group file are not set. The group
/// files have the names of the real installation (see the module text).
pub const VOCABULARY: [(&str, &str); 16] = [
    ("EDCoPilotSystem1.txt", "[Pellmar; Pelmar; Pellmar Quay]"),
    ("EDCoPilotSystem2.txt", "[Vellin;]"),
    ("EDCoPilotSystem3.txt", "[Vellan;]"),
    ("EDCoPilotSystem4.txt", "[KTL 3081;]"),
    ("EDCoPilotSystem5.txt", "[Ollin; Ollin Light]"),
    ("EDCoPilotSystem6.txt", "[Seven Gulls;]"),
    ("EDCoPilotSystem7.txt", "[Morrow;]"),
    ("EDCoPilotSystem8.txt", "[17 Skerry;]"),
    ("EDCoPilotSystem9.txt", "[Reef 2207;]"),
    ("EDCoPilotSystem10.txt", "[Brackmoor Sound;]"),
    ("EDCoPilotSystem11.txt", "Not set - harbour 11"),
    ("EDCoPilotSystem12.txt", "\u{feff}[Tarn Ness;]\r\n"),
    ("EDCoPilotBookmarkGrp1.txt", "Marta Quillon"),
    ("EDCoPilotBookmarkGrp2.txt", "Oskar Penhale"),
    ("EDCoPilotBookmarkGrp3.txt", "Not set"),
    ("EDCoPilotBookmarkGrp4.txt", "Ide Varrow"),
];

/// Adds commands and gives the bytes of a `.vap` file. The output is the
/// same for the same calls: the command ids come from a counter, and the
/// writer uses no time and no random value.
#[derive(Debug, Default, Clone)]
pub struct ProfileBuilder {
    commands: Vec<Vec<u8>>,
    category: Option<String>,
    count: u32,
}

impl ProfileBuilder {
    pub fn new() -> ProfileBuilder {
        ProfileBuilder::default()
    }

    /// The category of the commands that the next calls add.
    pub fn category(&mut self, name: &str) -> &mut Self {
        self.category = Some(name.to_string());
        self
    }

    /// The next commands have no category.
    pub fn no_category(&mut self) -> &mut Self {
        self.category = None;
        self
    }

    /// A full command that writes one fixed token. A sound before the
    /// write, as in a real command.
    pub fn fixed(&mut self, phrase: &str, token: &str) -> &mut Self {
        self.full(phrase, vec![sound(), write(token)])
    }

    /// A full command that takes the number of the spoken phrase into
    /// `var`: `LastCMD` = `{CMD}`, `var` = `{TXTNUM:LastCMD}`, then the
    /// write.
    pub fn number(&mut self, phrase: &str, var: &str, token: &str) -> &mut Self {
        self.full(
            phrase,
            vec![
                sound(),
                set_text("LastCMD", "{CMD}"),
                set_text(var, "{TXTNUM:LastCMD}"),
                write(token),
            ],
        )
    }

    /// A full command that reads the system slot file `file` with the
    /// system-slot parse, then writes the token.
    pub fn slot(&mut self, phrase: &str, file: u32, token: &str) -> &mut Self {
        let mut actions = slot_parse(file);
        actions.push(write(token));
        self.full(phrase, actions)
    }

    /// A full command that sets the constant `var` = `value`, then writes
    /// the token.
    pub fn constant(&mut self, phrase: &str, var: &str, value: &str, token: &str) -> &mut Self {
        self.full(phrase, vec![sound(), set_text(var, value), write(token)])
    }

    /// A composite prefix of `group` with the literal sets `sets`.
    pub fn prefix(&mut self, group: &str, phrase: &str, sets: &[(&str, &str)]) -> &mut Self {
        let mut actions = vec![sound()];
        actions.extend(sets.iter().map(|(n, v)| set_text(n, v)));
        self.raw(Some(phrase), 1, Some(group), actions)
    }

    /// A composite suffix of `group` in the known suffix shape. Each rule
    /// is (text that the spoken prefix contains, variable, value). The
    /// variable of the first rule is the one that the "has not been set"
    /// block checks.
    pub fn suffix(
        &mut self,
        group: &str,
        phrase: &str,
        rules: &[(&str, &str, &str)],
        token: &str,
    ) -> &mut Self {
        let mut actions = vec![sound()];
        for (contains, var, value) in rules {
            actions.push(condition("{PREFIX}", contains, 6));
            actions.push(set_text(var, value));
            actions.push(end_condition());
        }
        let checked = rules.first().map_or("", |r| r.1);
        actions.extend([
            condition(checked, "", 9),
            say("The trade side is not set."),
            super::test_support::else_branch(),
            write(token),
            end_condition(),
        ]);
        self.raw(Some(phrase), 2, Some(group), actions)
    }

    /// A full command with the given actions.
    pub fn full(&mut self, phrase: &str, actions: Vec<Vec<u8>>) -> &mut Self {
        self.raw(Some(phrase), 0, None, actions)
    }

    /// A command with any phrase, command type, group, and actions. The
    /// actions come from [`super::test_support`].
    pub fn raw(
        &mut self,
        phrase: Option<&str>,
        command_type: i32,
        group: Option<&str>,
        actions: Vec<Vec<u8>>,
    ) -> &mut Self {
        self.count += 1;
        let mut f = command_fields(phrase.unwrap_or(""), actions, command_type, group);
        f[0] = ProfileBuilder::id_of(self.count as usize).to_vec();
        if phrase.is_none() {
            f[1] = null_str();
        }
        f[7] = self.category.as_deref().map_or_else(null_str, str_field);
        self.commands.push(record(f));
        self
    }

    /// The id of the command number `n` (the first command is 1).
    pub fn id_of(n: usize) -> [u8; 16] {
        let mut id = [0u8; 16];
        id[..4].copy_from_slice(b"FXTR");
        id[12..].copy_from_slice(&(n as u32).to_le_bytes());
        id
    }

    /// The number of commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// The inflated profile record.
    pub fn inflated(&self) -> Vec<u8> {
        profile(self.commands.clone())
    }

    /// The bytes of the `.vap` file (raw DEFLATE of the profile record).
    pub fn build(&self) -> Vec<u8> {
        deflate(&self.inflated())
    }
}

const FLAG_COLOURS: [&str; 10] = [
    "blue", "red", "green", "yellow", "white", "black", "orange", "purple", "grey", "brown",
];
const LAMPS: [&str; 8] = [
    "pier", "beacon", "stern", "mast", "gate", "quay", "tower", "harbour",
];
const ROOMS: [&str; 12] = [
    "chart", "rope", "crew", "signal", "weather", "mooring", "ledger", "keeper", "beacon", "ferry",
    "lantern", "net",
];
const STATIONS: [&str; 8] = [
    "lamp",
    "quay",
    "boathouse",
    "bell tower",
    "signal mast",
    "store",
    "slipway",
    "net shed",
];
const DAYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];
const COMPASS: [&str; 8] = [
    "north",
    "north east",
    "east",
    "south east",
    "south",
    "south west",
    "west",
    "north west",
];

/// The constant commands: INI key of the name, constant system.
const KEEPERS: [(&str, &str); 8] = [
    ("OLDMILL", "Dunmore"),
    ("TAMFERRY", "Brackwater"),
    ("ROSEQUAY", "Ollin"),
    ("HALVARD", "Vennick"),
    ("NORTHMOLE", "Skerry 409"),
    ("WIDEPEAK", "Kessa"),
    ("LOSTKEEPER", "Graymouth"),
    ("KESTON", "Keston"),
];

/// The suffix items with a fixed phrase: spoken name, token name.
const PLAIN_ITEMS: [(&str, &str); 8] = [
    ("cork", "Cork"),
    ("twine", "Twine"),
    ("canvas", "Canvas"),
    ("fish nets", "Fish Nets"),
    ("tar", "Tar"),
    ("wax", "Wax"),
    ("pitch", "Pitch"),
    ("sailcloth", "Sailcloth"),
];

/// The suffix items with a runtime-grammar phrase: INI list key, token name.
const LIST_ITEMS: [(&str, &str); 4] = [
    ("ROPE", "Rope"),
    ("SALT", "Salt"),
    ("TIMBER", "Timber"),
    ("LAMPOIL", "Lamp Oil"),
];

/// Upper camel case of the words of `text`.
fn camel(text: &str) -> String {
    text.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect()
}

/// The builder with all the commands of the fixture.
pub fn fixture_builder() -> ProfileBuilder {
    let mut b = ProfileBuilder::new();
    lamp_room(&mut b);
    flags_and_signals(&mut b);
    bells_and_horns(&mut b);
    harbour_traffic(&mut b);
    cargo_market(&mut b);
    cargo_hold(&mut b);
    tide_and_weather(&mut b);
    logbook(&mut b);
    left_out(&mut b);
    b
}

fn lamp_room(b: &mut ProfileBuilder) {
    b.category("Lamp room")
        .number(
            "lamp brightness [0..100] [percent;]",
            "Level",
            "Prefs:LampBrightness=\"{TXT:Level}\"",
        )
        .number(
            "{TXT:EDCP_HORNVOLUME}",
            "Volume",
            "Prefs:HornVolume=\"{TXT:Volume}\"",
        )
        .fixed("lamp on", "Lamp:All:On")
        .fixed("lamp off", "Lamp:All:Off");
    for lamp in LAMPS {
        let name = camel(lamp);
        b.fixed(
            &format!("light the {lamp} lamp"),
            &format!("Lamp:{name}:On"),
        )
        .fixed(
            &format!("douse the {lamp} lamp"),
            &format!("Lamp:{name}:Off"),
        );
    }
    // A token variable that no action of the command sets: the command
    // needs a value that the phrase does not give.
    b.fixed("set lamp colour", "Lamp:Colour={TXT:LampColour}")
        // A working command with the phrase of the command above as one
        // alternative: that text gives no command (`shadowed_phrases`).
        .fixed("[paint the lamp; set lamp colour]", "Lamp:Paint")
        // "cell" is a word of the phrase: the sound-alike "cell" -> "sell"
        // is not tried.
        .fixed("lamp cell spares", "Spares:Lamp Cell")
        // Two commands that one text matches with different tokens.
        .fixed("close the shutter", "Shutter:Close")
        .fixed("[close; shut] the shutter", "LampShutter:Close");
}

fn flags_and_signals(b: &mut ProfileBuilder) {
    b.category("Flags and signals");
    for colour in FLAG_COLOURS {
        let name = camel(colour);
        b.fixed(
            &format!("raise the {colour} flag"),
            &format!("Flag:{name}:Up"),
        )
        .fixed(
            &format!("lower the {colour} flag"),
            &format!("Flag:{name}:Down"),
        );
    }
    b.fixed(
        "[raise; hoist] the storm [cone; signal] [now;]",
        "Signal:StormCone",
    )
    .number(
        "hoist pennant [1..12]",
        "PennantNo",
        "Pennant:{TXT:PennantNo}",
    );
}

fn bells_and_horns(b: &mut ProfileBuilder) {
    b.category("Bells and horns")
        .fixed("ring bell", "Bell:Ring")
        .fixed("ring bell twice", "Bell:RingTwice")
        .fixed("sound the noon gun", "Gun:Noon\r\n")
        .fixed(
            "toll the chapel bell; strike the chapel bell",
            "Bell:Chapel",
        )
        .number(
            "ring the bell [1..12] times",
            "Rings",
            "Bell:Times={TXT:Rings}",
        )
        .fixed("belay", "Belay");
    // Execute after the write, by id: the target only plays a sound. (A
    // name can come from the phrase of a runtime grammar, so the builder
    // does not resolve a name in this profile.)
    let horn = ProfileBuilder::id_of(b.len() + 2);
    b.full(
        "sound the horn",
        vec![write("Horn:Blast"), exec_by_id(horn)],
    )
    .full("horn blast effect", vec![sound()]);
}

fn harbour_traffic(b: &mut ProfileBuilder) {
    b.category("Harbour traffic");
    let sail = "HarbourAt:{TXT:SystemName}:sail to";
    let info = "HarbourInfo:{TXT:SystemName}:{CMD}";
    for n in 1..=SYSTEM_SLOTS {
        b.slot(&format!("{{TXT:EDCP_SAILTO}} {{TXT:System{n}}}"), n, sail)
            .slot(&format!("{{TXT:HarbourQuery}} {{TXT:System{n}}}"), n, info);
    }
    for (key, system) in KEEPERS {
        b.constant(
            &format!("{{TXT:EDCP_SAILTO}} {{TXT:EDCP_{key}}}"),
            "SystemName",
            system,
            sail,
        );
    }
    b.constant(
        "{TXT:HarbourQuery} {TXT:EDCP_ROSEQUAY}",
        "SystemName",
        "Ollin",
        info,
    )
    .constant(
        "{TXT:HarbourQuery} {TXT:EDCP_NORTHMOLE}",
        "SystemName",
        "Skerry 409",
        info,
    )
    // A slot command with the sound-alike word "course" ("kurs").
    .slot(
        "lay course to {TXT:System11}",
        11,
        "PlotCourse:{TXT:SystemName}",
    );
    for n in 1..=GROUP_SLOTS {
        b.fixed(
            &format!("hail {{TXT:Group{n}}}"),
            &format!("Hail:{{TXT:Group{n}}}"),
        );
    }
    b.fixed("head for the lighthouse", "HeadForLighthouse")
        .fixed("{TXT:EDCP_HOMEPORT}", "SailHomePort")
        .fixed("lay course for home port", "PlotCourse:HomePort")
        .number(
            "show buoys within [1..50] thousand yards",
            "Yards",
            "ShowBuoysWithin={TXT:Yards}000",
        )
        .fixed(
            "[nearest berth;] [size;] [1..4;] [old; new; spare;] steam crane",
            "Berth:Steam Crane~{CMD}",
        )
        .fixed("steer the pilot boat", "PilotBoat:Steer")
        .fixed("pilot cutter mark two", "Vessel:CutterMk2")
        .fixed("{TXT:EDCP_GATECHIME}", "Gate:Chime");
}

fn cargo_market(b: &mut ProfileBuilder) {
    b.category("Cargo market");
    let g = MARKET_GROUP;
    b.prefix(
        g,
        "[give me the;] [dearest; cheapest] [sell; sale; buy; purchase] bid",
        &[("offer_scope", "Top:")],
    )
    .prefix(
        g,
        "[give me the;] dock [sell; buy]",
        &[("offer_scope", "Dock:")],
    )
    .prefix(
        g,
        "nearest [quay; harbour to;] [sell; buy; selling; buying]",
        &[("offer_scope", "Near:")],
    )
    .prefix(g, "ways of [selling; buying]", &[("offer_scope", "Ways:")])
    .prefix(g, "cargo report on", &[("offer_scope", "Report:")]);
    let rules = [
        ("buy", "TradeSide", "Buy"),
        ("sell", "TradeSide", "Sell"),
        ("purchas", "TradeSide", "Buy"),
        ("sale", "TradeSide", "Sell"),
    ];
    for (spoken, name) in PLAIN_ITEMS {
        b.suffix(
            g,
            &format!("[rate; offer;] [for;] {spoken}"),
            &rules,
            &format!("{{TXT:TradeSide}}{{TXT:offer_scope}}{name}"),
        );
    }
    for (key, name) in LIST_ITEMS {
        b.suffix(
            g,
            &format!("{{TXT:EDCP_RATE_{key}}}"),
            &rules,
            &format!("{{TXT:TradeSide}}{{TXT:offer_scope}}{name}"),
        )
        .fixed(
            &format!("{{TXT:EDCP_SEEK_{key}}}"),
            &format!("SetCargo:{name}"),
        );
    }
    b.fixed("sell at dock; dock sell", "FindDockSell")
        .fixed("buy at dock; dock buy", "FindDockBuy")
        .fixed("[find; seek;] cork", "SetCargo:Cork")
        .fixed("sort nets by length", "Prefs:SortNets=\"2\"")
        .fixed("sort stored crates by size", "Prefs:SortCrates=\"0\"");
}

fn cargo_hold(b: &mut ProfileBuilder) {
    b.category("Hold and stores")
        .number(
            "{TXT:EDCP_NETREMINDER}",
            "NetCount",
            "SetNetReminder:{TXT:NetCount}",
        )
        .number(
            "add lamp oil to the store by [1..50] barrels",
            "Barrels",
            "AddLampOil:{TXT:Barrels}",
        )
        .number(
            "set crate count to [1..99]",
            "Crates",
            "SetCrateCount:{TXT:Crates}",
        )
        .number(
            "remove crate [1..30]",
            "CrateNo",
            "RemoveCrate:{TXT:CrateNo}",
        )
        .fixed("dump the nets", "Nets:Dump")
        .number(
            "{TXT:EDCP_MOORING}",
            "MooringNo",
            "SelectMooring:{TXT:MooringNo}",
        )
        .number("{TXT:EDCP_WICKLAMP}", "WickNo", "WickLamp:{TXT:WickNo}")
        .number(
            "delete mooring [1..50]",
            "MooringNo",
            "DeleteMooring:{TXT:MooringNo}",
        )
        .number(
            "delete mooring group [1..10]",
            "GroupNo",
            "DeleteMooringGroup:{TXT:GroupNo}",
        )
        .number(
            "move the ferry to mooring [1..50]",
            "MooringNo",
            "FerryToMooring:{TXT:MooringNo}",
        )
        .number(
            "set the context to mooring [1..9] [0..9]",
            "MooringNo",
            "MooringContext:{TXT:MooringNo}",
        )
        .number(
            "pick mooring [1..9] [0..9]",
            "MooringNo",
            "PickMooringVoice:{TXT:MooringNo}",
        )
        .number(
            "sail for buoy [1..9] [0..9]",
            "BuoyNo",
            "SailForBuoy:{TXT:BuoyNo}",
        );
    for station in STATIONS {
        let name = camel(station);
        b.fixed(
            &format!("crew to the {station}"),
            &format!("Crew:To:{name}"),
        )
        .fixed(
            &format!("crew from the {station}"),
            &format!("Crew:From:{name}"),
        );
    }
}

fn tide_and_weather(b: &mut ProfileBuilder) {
    b.category("Tide and weather");
    for day in DAYS {
        let name = camel(day);
        b.fixed(
            &format!("tide table for {day}"),
            &format!("TideTable:{name}"),
        )
        .fixed(
            &format!("tide height for {day}"),
            &format!("TideHeight:{name}"),
        );
    }
    for point in COMPASS {
        let name = camel(point);
        b.fixed(
            &format!("wind from the {point}"),
            &format!("WindFrom:{name}"),
        )
        .fixed(
            &format!("gusts from the {point}"),
            &format!("GustsFrom:{name}"),
        );
    }
    b.number(
        "storm warning level [1..5]",
        "StormLevel",
        "StormWarning:{TXT:StormLevel}",
    )
    .number(
        "high water in [1..12] hours",
        "Hours",
        "HighWaterIn:{TXT:Hours}",
    )
    // The sound-alike "higher" -> "highest".
    .fixed("highest water mark", "TideMark:Highest");
}

fn logbook(b: &mut ProfileBuilder) {
    b.category("Logbook")
        .fixed("back to the tide log", "OpenTideLogPanel")
        .fixed("{TXT:EDCP_TIDELOG}", "OpenTideLogPanel")
        .fixed("help tide log", "Guide:Panel:TideLog")
        .fixed("next tide log", "TideLog:Next")
        .fixed("lock current ledger", "LedgerLock:Current")
        .fixed("swing the lock gates", "LockGates:Swing")
        .fixed("lock the boathouse", "Boathouse:Lock={TXT:BoathouseState}")
        .fixed(
            "unlock the boathouse",
            "Boathouse:Unlock={TXT:BoathouseState}",
        )
        .fixed("higher tide alerts", "Alerts:HigherTide")
        // "log" at the start of a phrase of several words.
        .fixed("log the wind speed", "LogWind:Speed")
        // A two-word "lock" phrase that needs a value.
        .fixed("lock lantern", "LanternLock={TXT:LanternState}")
        // The sound-alike variant of "lock lantern" is this phrase.
        .fixed("log lantern", "LanternLog:Open")
        // One fuzzy word from the left-out command "test the lamp".
        .fixed("test the lamps", "Lamp:TestAll");
    for room in ROOMS {
        let name = camel(room);
        b.fixed(
            &format!("open the {room} room"),
            &format!("OpenRoom:{name}"),
        )
        .fixed(&format!("help {room} room"), &format!("Guide:Room:{name}"));
    }
}

/// The commands that the builder leaves out, one or more of each class.
fn left_out(b: &mut ProfileBuilder) {
    b.no_category()
        // No write. It also sets the profile variable `HarbourQuery`.
        .full(
            "load harbour queries",
            vec![sound(), set_text("HarbourQuery", HARBOUR_QUERY)],
        )
        // Two writes.
        .full("ring all bells", vec![write("Bell:One"), write("Bell:Two")])
        // Maintenance: a key press, and a write in a condition block.
        .full(
            "reset the lamp profile",
            vec![write("LampProfile:Reset"), key_press()],
        )
        .full(
            "rebuild the lamp profile",
            vec![
                condition("LampProfileReady", "yes", 0),
                write("LampProfile:Rebuild"),
                end_condition(),
            ],
        )
        // Not understood: an action type that the table does not list.
        .full(
            "polish the lens",
            vec![plain_action(99), write("Lens:Polish")],
        )
        // Not understood: the command sets a part of its token by a copy.
        .full(
            "report the wind",
            vec![set_text_copy("Wind", "WindVane"), write("Wind:{TXT:Wind}")],
        )
        // No write: it plays a sound only. Its sound-alike variant ("lock"
        // -> "log") is the working phrase "log the wind speed".
        .full("lock the wind speed", vec![sound()]);
    // Not understood: the target of the execute action has code that is
    // not reviewed. The target itself writes nothing.
    let check = ProfileBuilder::id_of(b.len() + 2);
    b.full("test the lamp", vec![write("Lamp:Test"), exec_by_id(check)])
        .full(
            "lamp self check",
            vec![inline_code("// invented code of the fixture\nreturn;")],
        );
    // An orphan suffix: no prefix has its group.
    b.category("Cargo market").suffix(
        "lost cargo",
        "[for;] driftwood",
        &[("sell", "TradeSide", "Sell")],
        "{TXT:TradeSide}Driftwood",
    );
}

/// The standard invented profile: the bytes of a `.vap` file.
pub fn fixture_profile() -> Vec<u8> {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| fixture_builder().build()).clone()
}

/// The commands that [`build_map`] gives for [`fixture_profile`]. The
/// phrases still have their `{TXT:EDCP_...}` and profile variables, as
/// the builder gives them.
pub fn fixture_commands() -> Vec<Command> {
    static COMMANDS: OnceLock<Vec<Command>> = OnceLock::new();
    COMMANDS
        .get_or_init(|| {
            build_map(&fixture_profile())
                .expect("the fixture profile builds")
                .commands
        })
        .clone()
}

/// The counts of the fixture that the tests of the other modules and
/// crates compare with. [`FIXTURE_COUNTS`] is the only place that has them
/// as numbers. Thus a new fixture command needs one change of a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureCounts {
    /// The commands of the profile, also the left-out commands.
    pub commands_in_profile: usize,
    /// The records of the build (the commands that are not left out).
    pub records: usize,
    pub left_out: usize,
    /// The records that the number check of the runtime grammar takes out.
    pub removed_by_number_check: usize,
    /// The records after the number check: the loaded commands.
    pub loaded: usize,
    /// The loaded prefixes and suffixes of composite commands.
    pub composite_parts: usize,
    /// The blocking phrases of the build (`BuiltMap::blocking`).
    pub blocking: usize,
    /// The blocking phrases after the load: those of the build and those of
    /// the commands that the number check takes out, with the runtime
    /// grammar, without the skipped ones.
    pub blocking_loaded: usize,
    /// The blocking phrases that do not resolve or do not parse.
    pub blocking_skipped: usize,
    pub vocabulary_files: usize,
    /// The names of all vocabulary files. A "Not set" file gives no name.
    pub vocabulary_names: usize,
}

/// The pinned counts of the fixture. The test
/// `test_fixture_counts_are_pinned` of `fixture_tests.rs` computes them
/// from the build of [`fixture_profile`], the runtime grammar of
/// [`VOICE_COMMANDS_INI`] and the files of [`VOCABULARY`], and compares.
pub const FIXTURE_COUNTS: FixtureCounts = FixtureCounts {
    commands_in_profile: 237,
    records: 226,
    left_out: 11,
    removed_by_number_check: 1,
    loaded: 225,
    composite_parts: 17,
    blocking: 10,
    blocking_loaded: 11,
    blocking_skipped: 0,
    vocabulary_files: 16,
    vocabulary_names: 17,
};

/// [`FIXTURE_COUNTS`].
pub fn fixture_counts() -> FixtureCounts {
    FIXTURE_COUNTS
}

/// Writes an invented EDCoPilot folder into `dir`: the profile
/// `EDCoPilot-Profile.vap`, `working\EDCoPilot.VoiceCommands.ini`, and the
/// vocabulary files in `Dynamic var files`. It writes nothing outside
/// `dir`. It panics when a write fails (test code).
pub fn write_fixture_edcopilot(dir: &Path) {
    let working = dir.join("working");
    let vars = dir.join("Dynamic var files");
    std::fs::create_dir_all(&working).expect("make the working folder");
    std::fs::create_dir_all(&vars).expect("make the vocabulary folder");
    std::fs::write(dir.join("EDCoPilot-Profile.vap"), fixture_profile())
        .expect("write the profile");
    std::fs::write(
        working.join("EDCoPilot.VoiceCommands.ini"),
        VOICE_COMMANDS_INI,
    )
    .expect("write the voice commands file");
    for (name, text) in VOCABULARY {
        std::fs::write(vars.join(name), text).expect("write a vocabulary file");
    }
}

#[cfg(test)]
#[path = "fixture_tests.rs"]
mod tests;
