use crate::cli::ShellKind;
use crate::config::Config;

const ZSH: &str = include_str!("../shell/undo.zsh");
const BASH: &str = include_str!("../shell/undo.bash");
const FISH: &str = include_str!("../shell/undo.fish");

const BUILTINS: [&str; 9] = [
    "mv", "cp", "rm", "mkdir", "rmdir", "chmod", "chown", "ln", "rename",
];

pub fn snippet(shell: ShellKind) -> &'static str {
    match shell {
        ShellKind::Zsh => ZSH,
        ShellKind::Bash => BASH,
        ShellKind::Fish => FISH,
    }
}

/// A shell command/function name is safe to splice into the snippet if it is a
/// plausible command name and not one of the nine built-ins (those are already
/// wrapped by the static snippet).
fn is_safe_alias(name: &str) -> bool {
    !name.is_empty()
        && !BUILTINS.contains(&name)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Aliases whose target is a known built-in, sorted and de-duplicated.
fn alias_names(config: &Config) -> Vec<String> {
    let mut names: Vec<String> = config
        .plugins
        .iter()
        .filter(|(name, target)| is_safe_alias(name) && BUILTINS.contains(&target.as_str()))
        .map(|(name, _)| name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The full integration snippet: the static wrappers for the nine built-ins,
/// plus dynamic wrappers for any configured plugin aliases.
pub fn emit(shell: ShellKind) -> String {
    let config = Config::load().unwrap_or_default();
    let names = alias_names(&config);
    let mut out = snippet(shell).to_string();
    if names.is_empty() {
        return out;
    }
    let list = names.join(" ");
    match shell {
        ShellKind::Zsh => {
            out.push_str(&format!(
                "\nfor _undo_cmd in {list}; do\n  eval \"function ${{_undo_cmd}} {{ _undo_wrap ${{_undo_cmd}} \\\"\\$@\\\" }}\"\ndone\nunset _undo_cmd\n"
            ));
        }
        ShellKind::Bash => {
            out.push_str(&format!(
                "\nfor _undo_cmd in {list}; do\n  eval \"${{_undo_cmd}}() {{ _undo_wrap ${{_undo_cmd}} \\\"\\$@\\\"; }}\"\ndone\nunset _undo_cmd\n"
            ));
        }
        ShellKind::Fish => {
            out.push_str(&format!(
                "\nfor _undo_cmd in {list}\n    eval \"function $_undo_cmd --wraps $_undo_cmd; _undo_wrap $_undo_cmd \\$argv; end\"\nend\nset -e _undo_cmd\n"
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_wrap_all_nine_commands() {
        for shell in [ShellKind::Zsh, ShellKind::Bash, ShellKind::Fish] {
            let s = snippet(shell);
            for cmd in [
                "mv", "cp", "rm", "mkdir", "rmdir", "chmod", "chown", "ln", "rename",
            ] {
                assert!(s.contains(cmd), "{shell:?} snippet misses {cmd}");
            }
            assert!(
                s.contains("125"),
                "{shell:?} snippet misses the fallback protocol"
            );
            assert!(
                s.contains("UNDO_ACTIVE"),
                "{shell:?} snippet misses the recursion guard"
            );
        }
    }
}
