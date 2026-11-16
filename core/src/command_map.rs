//! One command of the map: the record that the profile builder gives.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CommandClass {
    #[default]
    Fixed,
    DynamicName,
    DynamicValue,
}

/// The VoiceAttack command type. A prefix and a suffix of the same composite
/// group are spoken together as one sentence. Neither is a command alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CommandType {
    #[default]
    Full,
    Prefix,
    Suffix,
}

/// One "if `{PREFIX}` contains `contains`, set the variables in `set`" action
/// of a suffix command.
#[derive(Debug, Clone, PartialEq)]
pub struct PrefixRule {
    pub contains: String,
    pub set: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Command {
    pub phrase: Option<String>,
    pub token_template: String,
    pub class: CommandClass,
    pub var_source: Option<String>,
    pub category: Option<String>,
    /// Placeholder values the command always writes: variable name to value.
    /// Example: "sail to the old mill" has `SystemName` = `Dunmore`.
    pub constants: HashMap<String, String>,
    /// Placeholder variables that take the number in the spoken phrase.
    /// Example: "show buoys within 12 thousand yards" gives `Yards` = `12`.
    pub number_vars: Vec<String>,
    /// Full command, or a part of a composite command.
    pub command_type: CommandType,
    /// The composite group of a prefix or a suffix.
    pub composite_group: Option<String>,
    /// Prefix only: the variables that the prefix sets, name to value.
    /// Example: `offer_scope` = `Dock:`.
    pub sets: HashMap<String, String>,
    /// Suffix only: the conditions on the spoken prefix, in action order.
    pub prefix_rules: Vec<PrefixRule>,
    /// Names of a fixed name slot that `resolve_runtime_grammar` made: slot
    /// name to the spoken names. The builder leaves it empty.
    pub fixed_vocab: HashMap<String, Vec<String>>,
}

impl Command {
    /// True for a prefix or a suffix of a composite command.
    pub fn is_composite_part(&self) -> bool {
        self.command_type != CommandType::Full
    }

    /// The token template with each constant put in the place of its
    /// `{TXT:name}` placeholder. Two commands with the same raw template and
    /// different constants are different commands.
    pub fn effective_template(&self) -> String {
        let mut template = self.token_template.clone();
        for (name, value) in &self.constants {
            template = template.replace(&format!("{{TXT:{name}}}"), value);
        }
        template
    }
}

#[cfg(test)]
mod tests {
    use crate::profile::fixture::fixture_commands;

    /// The constant goes into the token in the place of its placeholder.
    #[test]
    fn test_effective_template_puts_the_constants_into_the_token() {
        let commands = fixture_commands();
        let old_mill = commands
            .iter()
            .find(|c| c.phrase.as_deref() == Some("{TXT:EDCP_SAILTO} {TXT:EDCP_OLDMILL}"))
            .unwrap();
        assert_eq!(
            old_mill.constants.get("SystemName").map(String::as_str),
            Some("Dunmore")
        );
        assert_eq!(old_mill.effective_template(), "HarbourAt:Dunmore:sail to");
    }

    #[test]
    fn test_effective_template_without_constants_is_the_template() {
        let commands = fixture_commands();
        let slot = commands
            .iter()
            .find(|c| c.phrase.as_deref() == Some("{TXT:EDCP_SAILTO} {TXT:System1}"))
            .unwrap();
        assert!(slot.constants.is_empty());
        assert_eq!(slot.effective_template(), slot.token_template);
    }
}
