#[derive(Debug, Clone, PartialEq)]
pub enum ModeSpec {
    Octal(u32),
    Clauses(Vec<Clause>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Clause {
    whos: WhoSet,
    ops: Vec<OpSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct WhoSet {
    u: bool,
    g: bool,
    o: bool,
    explicit_all: bool,
    empty: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct OpSpec {
    op: char,
    perms: PermBits,
}

#[derive(Debug, Clone, PartialEq)]
enum PermBits {
    Letters {
        r: bool,
        w: bool,
        x: bool,
        big_x: bool,
        s: bool,
        t: bool,
    },
    Copy(char),
}

pub fn parse(s: &str) -> Result<ModeSpec, String> {
    if s.is_empty() {
        return Err("empty mode".into());
    }
    if s.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        if s.len() > 4 {
            return Err(format!("octal mode '{s}' has more than 4 digits"));
        }
        return Ok(ModeSpec::Octal(
            u32::from_str_radix(s, 8).expect("digits checked"),
        ));
    }
    let mut clauses = Vec::new();
    for part in s.split(',') {
        clauses.push(parse_clause(part)?);
    }
    Ok(ModeSpec::Clauses(clauses))
}

fn parse_clause(part: &str) -> Result<Clause, String> {
    let chars: Vec<char> = part.chars().collect();
    let mut i = 0;
    let mut whos = WhoSet {
        u: false,
        g: false,
        o: false,
        explicit_all: false,
        empty: false,
    };
    while i < chars.len() && matches!(chars[i], 'u' | 'g' | 'o' | 'a') {
        match chars[i] {
            'u' => whos.u = true,
            'g' => whos.g = true,
            'o' => whos.o = true,
            'a' => {
                whos.u = true;
                whos.g = true;
                whos.o = true;
                whos.explicit_all = true;
            }
            _ => unreachable!(),
        }
        i += 1;
    }
    if !(whos.u || whos.g || whos.o) {
        whos = WhoSet {
            u: true,
            g: true,
            o: true,
            explicit_all: false,
            empty: true,
        };
    }

    let mut ops = Vec::new();
    while i < chars.len() {
        let op = chars[i];
        if !matches!(op, '+' | '-' | '=') {
            return Err(format!("invalid character '{op}' in mode '{part}'"));
        }
        i += 1;
        if i < chars.len()
            && matches!(chars[i], 'u' | 'g' | 'o')
            && (i + 1 == chars.len() || matches!(chars[i + 1], '+' | '-' | '='))
        {
            ops.push(OpSpec {
                op,
                perms: PermBits::Copy(chars[i]),
            });
            i += 1;
            continue;
        }
        let mut letters = PermBits::Letters {
            r: false,
            w: false,
            x: false,
            big_x: false,
            s: false,
            t: false,
        };
        if let PermBits::Letters {
            r,
            w,
            x,
            big_x,
            s,
            t,
        } = &mut letters
        {
            while i < chars.len() && matches!(chars[i], 'r' | 'w' | 'x' | 'X' | 's' | 't') {
                match chars[i] {
                    'r' => *r = true,
                    'w' => *w = true,
                    'x' => *x = true,
                    'X' => *big_x = true,
                    's' => *s = true,
                    't' => *t = true,
                    _ => unreachable!(),
                }
                i += 1;
            }
        }
        ops.push(OpSpec { op, perms: letters });
    }
    if ops.is_empty() {
        return Err(format!("mode clause '{part}' has no operator"));
    }
    Ok(Clause { whos, ops })
}

pub fn apply(spec: &ModeSpec, old: u32, is_dir: bool, umask: u32) -> u32 {
    match spec {
        ModeSpec::Octal(m) => *m & 0o7777,
        ModeSpec::Clauses(clauses) => {
            let mut mode = old & 0o7777;
            for clause in clauses {
                mode = apply_clause(clause, mode, is_dir, umask);
            }
            mode
        }
    }
}

fn class_shift(class: char) -> u32 {
    match class {
        'u' => 6,
        'g' => 3,
        _ => 0,
    }
}

fn special_bit(class: char) -> u32 {
    match class {
        'u' => 0o4000,
        'g' => 0o2000,
        _ => 0o1000,
    }
}

fn apply_clause(clause: &Clause, mut mode: u32, is_dir: bool, umask: u32) -> u32 {
    let classes = [
        ('u', clause.whos.u),
        ('g', clause.whos.g),
        ('o', clause.whos.o),
    ];
    for op in &clause.ops {
        for (class, enabled) in classes {
            if !enabled {
                continue;
            }
            let shift = class_shift(class);
            let (mut rwx, special) = match &op.perms {
                PermBits::Letters {
                    r,
                    w,
                    x,
                    big_x,
                    s,
                    t,
                } => {
                    let mut bits = 0u32;
                    if *r {
                        bits |= 4;
                    }
                    if *w {
                        bits |= 2;
                    }
                    if *x || (*big_x && (is_dir || mode & 0o111 != 0)) {
                        bits |= 1;
                    }
                    let mut sp = 0u32;
                    if *s && matches!(class, 'u' | 'g') {
                        sp |= special_bit(class);
                    }
                    if *t && class == 'o' {
                        sp |= 0o1000;
                    }
                    (bits, sp)
                }
                PermBits::Copy(src) => ((mode >> class_shift(*src)) & 7, 0),
            };
            if clause.whos.empty {
                rwx &= !(umask >> shift) & 7;
            }
            let bits = (rwx << shift) | special;
            match op.op {
                '+' => mode |= bits,
                '-' => mode &= !bits,
                '=' => {
                    let mut clear_rwx = 7u32;
                    if clause.whos.empty {
                        clear_rwx &= !(umask >> shift) & 7;
                    }
                    let clear = (clear_rwx << shift) | special_bit(class);
                    mode = (mode & !clear) | bits;
                }
                _ => unreachable!(),
            }
        }
    }
    mode
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply_str(s: &str, old: u32, is_dir: bool, umask: u32) -> u32 {
        apply(&parse(s).unwrap(), old, is_dir, umask)
    }

    #[test]
    fn octal_modes() {
        assert_eq!(apply_str("755", 0o644, false, 0o022), 0o755);
        assert_eq!(apply_str("0644", 0o777, false, 0o022), 0o644);
        assert_eq!(apply_str("4755", 0, false, 0o022), 0o4755);
        assert!(parse("77777").is_err());
        assert!(parse("8").is_err());
    }

    #[test]
    fn plus_minus_equals() {
        assert_eq!(apply_str("u+x", 0o644, false, 0o022), 0o744);
        assert_eq!(apply_str("go-w", 0o666, false, 0o022), 0o644);
        assert_eq!(apply_str("a=r", 0o777, false, 0o022), 0o444);
        assert_eq!(apply_str("u=rwx,g=rx,o=", 0o644, false, 0o022), 0o750);
        assert_eq!(apply_str("ug+rw", 0o400, false, 0o022), 0o660);
    }

    #[test]
    fn omitted_who_honors_umask() {
        assert_eq!(apply_str("+x", 0o644, false, 0o022), 0o755);
        assert_eq!(apply_str("+x", 0o644, false, 0o027), 0o754);
        assert_eq!(apply_str("a+x", 0o644, false, 0o027), 0o755);
    }

    #[test]
    fn big_x_only_for_dirs_or_executables() {
        assert_eq!(apply_str("a+X", 0o644, false, 0), 0o644);
        assert_eq!(apply_str("a+X", 0o644, true, 0), 0o755);
        assert_eq!(apply_str("a+X", 0o744, false, 0), 0o755);
    }

    #[test]
    fn setuid_setgid_sticky() {
        assert_eq!(apply_str("u+s", 0o755, false, 0o022), 0o4755);
        assert_eq!(apply_str("g+s", 0o755, true, 0o022), 0o2755);
        assert_eq!(apply_str("ug+s", 0o755, false, 0o022), 0o6755);
        assert_eq!(apply_str("o+t", 0o777, true, 0o022), 0o1777);
        assert_eq!(apply_str("+t", 0o777, true, 0o022), 0o1777);
        assert_eq!(apply_str("u-s", 0o4755, false, 0o022), 0o755);
        assert_eq!(apply_str("u=rw", 0o4755, false, 0o022), 0o655);
    }

    #[test]
    fn class_copy() {
        assert_eq!(apply_str("g=u", 0o750, false, 0o022), 0o770);
        assert_eq!(apply_str("o=g", 0o750, false, 0o022), 0o755);
        assert_eq!(apply_str("u+g", 0o470, false, 0o022), 0o770);
    }

    #[test]
    fn multiple_ops_in_one_clause() {
        assert_eq!(apply_str("u+r-w", 0o200, false, 0o022), 0o400);
        assert_eq!(apply_str("u=rw+x", 0o000, false, 0o022), 0o700);
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "u", "z+x", "u~x", "u+q", "--reference=/x", "u+x,"] {
            assert!(parse(bad).is_err(), "should reject {bad:?}");
        }
    }
}
