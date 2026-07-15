use crate::cli::ShellKind;

const ZSH: &str = include_str!("../shell/undo.zsh");
const BASH: &str = include_str!("../shell/undo.bash");
const FISH: &str = include_str!("../shell/undo.fish");

pub fn snippet(shell: ShellKind) -> &'static str {
    match shell {
        ShellKind::Zsh => ZSH,
        ShellKind::Bash => BASH,
        ShellKind::Fish => FISH,
    }
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
