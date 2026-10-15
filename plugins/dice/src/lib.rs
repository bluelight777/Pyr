//! Dice: roll dice in chat with /roll (like `/roll 2d6+1`), or from its panel.

use ingle_plugin::*;

struct Dice;

/// The most dice in one roll, and the most sides a die can have.
const MAX_DICE: u64 = 100;
const MAX_SIDES: u64 = 1000;

/// One part of a roll: `count` dice of `sides` sides (`sides` 0: a plain number), added or taken
/// away.
#[derive(Debug, PartialEq)]
struct Term {
    count: u64,
    sides: u64,
    minus: bool,
}

/// Reads "2d6+1", "d20", "3d8-2+d4"; empty means a d20.
fn parse(expr: &str) -> Result<Vec<Term>, String> {
    let s: String = expr.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
    let s = if s.is_empty() { "d20".to_string() } else { s };
    let mut terms = Vec::new();
    let mut rest = s.as_str();
    let mut minus = false;
    loop {
        let end = rest.find(['+', '-']).unwrap_or(rest.len());
        let part = &rest[..end];
        if part.is_empty() {
            return Err(format!("I can't read \"{}\". Try /roll 2d6+1.", expr.trim()));
        }
        let unreadable = || format!("I can't read \"{}\". Try /roll 2d6+1.", expr.trim());
        let term = match part.split_once('d') {
            Some((n, sides)) => {
                let count = if n.is_empty() { 1 } else { n.parse().map_err(|_| unreadable())? };
                let sides: u64 = sides.parse().map_err(|_| unreadable())?;
                if count == 0 || count > MAX_DICE {
                    return Err(format!("Roll between 1 and {MAX_DICE} dice at a time."));
                }
                if !(2..=MAX_SIDES).contains(&sides) {
                    return Err(format!("A die has between 2 and {MAX_SIDES} sides."));
                }
                Term { count, sides, minus }
            }
            None => Term { count: part.parse().map_err(|_| unreadable())?, sides: 0, minus },
        };
        terms.push(term);
        if terms.len() > 20 {
            return Err("That's a lot of parts. Keep it to 20.".into());
        }
        if end == rest.len() {
            break;
        }
        minus = rest.as_bytes()[end] == b'-';
        rest = &rest[end + 1..];
    }
    Ok(terms)
}

/// Rolls them: the total, and how it came out ("2d6+1: 4 + 3 + 1 = 8").
fn roll(ctx: &mut Ctx, expr: &str) -> Result<(i64, String), String> {
    let terms = parse(expr)?;
    let mut total: i64 = 0;
    let mut shown = Vec::new();
    let mut rolled_any = 0;
    for t in &terms {
        let sign = if t.minus { -1 } else { 1 };
        if t.sides == 0 {
            total += sign * t.count as i64;
            shown.push(format!("{}{}", if t.minus { "-" } else { "" }, t.count));
            continue;
        }
        let rolls: Vec<u64> = (0..t.count).map(|_| ctx.random(t.sides) + 1).collect();
        rolled_any += rolls.len();
        total += sign * rolls.iter().sum::<u64>() as i64;
        let list = rolls.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(" + ");
        shown.push(if t.minus { format!("-({list})") } else if rolls.len() > 1 && terms.len() > 1 { format!("({list})") } else { list });
    }
    let name: String = terms
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let sign = if t.minus { "-" } else if i > 0 { "+" } else { "" };
            if t.sides == 0 { format!("{sign}{}", t.count) } else if t.count == 1 { format!("{sign}d{}", t.sides) } else { format!("{sign}{}d{}", t.count, t.sides) }
        })
        .collect();
    let working = if rolled_any == 1 && terms.len() == 1 { String::new() } else { format!("{} = ", shown.join(" + ").replace("+ -", "- ")) };
    Ok((total, format!("🎲 {name}: {working}**{total}**")))
}

/// The last rolls (newest first), kept in the add-on's data.
fn remember(ctx: &mut Ctx, line: &str) {
    let mut last: Vec<Value> = ctx.store.get("last").and_then(|v| v.as_array().cloned()).unwrap_or_default();
    last.insert(0, json!(line));
    last.truncate(8);
    ctx.store = json!({ "last": last });
}

impl AddOn for Dice {
    fn info() -> Info {
        Info::new("Dice").command("roll", "Roll dice: /roll 2d6+1, /roll d20 (just /roll is a d20)")
    }

    fn command(ctx: &mut Ctx, _name: &str, args: &str) -> Result<String, String> {
        let (_, line) = roll(ctx, args)?;
        remember(ctx, &line);
        Ok(line)
    }

    fn panel(ctx: &mut Ctx) -> Panel {
        let die = |n: u64| Widget::Button { id: format!("d{n}"), label: format!("d{n}") };
        let mut p = Panel::new()
            .heading("Roll in chat")
            .row(vec![die(4), die(6), die(8), die(10)])
            .row(vec![die(12), die(20), die(100)])
            .input("custom", "Or type dice, like 3d6+2, and press Enter")
            .divider();
        let last: Vec<String> = ctx
            .store
            .get("last")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        if last.is_empty() {
            p = p.text("Rolls you make show here, and go to the channel you have open.");
        } else {
            p = p.text("Your last rolls:");
            for l in last {
                p = p.text(l.replace("**", "").replace("🎲 ", ""));
            }
        }
        p
    }

    fn event(ctx: &mut Ctx, id: &str, value: &str) -> Option<String> {
        let expr = if id == "custom" { value.to_string() } else { id.to_string() };
        match roll(ctx, &expr) {
            Ok((_, line)) => {
                remember(ctx, &line);
                Some(line)
            }
            Err(e) => {
                remember(ctx, &format!("({e})"));
                None
            }
        }
    }
}

export!(Dice);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_dice() {
        assert_eq!(parse("2d6+1").unwrap(), vec![Term { count: 2, sides: 6, minus: false }, Term { count: 1, sides: 0, minus: false }]);
        assert_eq!(parse("").unwrap(), vec![Term { count: 1, sides: 20, minus: false }]);
        assert_eq!(parse(" D8 - 2 ").unwrap(), vec![Term { count: 1, sides: 8, minus: false }, Term { count: 2, sides: 0, minus: true }]);
        for bad in ["2d", "d1", "0d6", "101d6", "d1001", "abc", "2d6+", "+3", "1d6++2"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rolls_add_up() {
        let mut ctx = Ctx { seed: 99, ..Default::default() };
        for _ in 0..200 {
            let (t, line) = roll(&mut ctx, "2d6+1").unwrap();
            assert!((3..=13).contains(&t), "{t}");
            assert!(line.starts_with("🎲 2d6+1: ") && line.ends_with(&format!("**{t}**")), "{line}");
            let (t, _) = roll(&mut ctx, "d4-10").unwrap();
            assert!((-9..=-6).contains(&t));
        }
        let (t, line) = roll(&mut ctx, "d20").unwrap();
        assert_eq!(line, format!("🎲 d20: **{t}**"));
    }

    #[test]
    fn the_panel_remembers_the_last_rolls() {
        let mut ctx = Ctx { seed: 1, ..Default::default() };
        for _ in 0..10 {
            Dice::event(&mut ctx, "d20", "");
        }
        assert_eq!(ctx.store["last"].as_array().unwrap().len(), 8);
        assert!(Dice::event(&mut ctx, "custom", "nonsense").is_none());
        let p = Dice::panel(&mut ctx);
        assert!(p.widgets.iter().any(|w| matches!(w, Widget::Text { text } if text.contains("can't read"))));
    }
}
