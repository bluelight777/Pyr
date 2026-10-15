//! A starting point for an Ingle add-on: a /hello command and a panel with a counter.
//! Rename it (Cargo.toml's name and [package.metadata.ingle]) and make it yours.

use ingle_plugin::*;

struct Hello;

impl AddOn for Hello {
    /// Its name, and the /commands it adds to chat.
    fn info() -> Info {
        Info::new("Hello").command("hello", "Say hello: /hello Mara")
    }

    /// `/hello Mara`: what's returned is sent as the person's message.
    fn command(ctx: &mut Ctx, _name: &str, args: &str) -> Result<String, String> {
        let who = args.trim();
        if who.len() > 60 {
            return Err("That's a long name. Try a shorter one.".into());
        }
        let who = if who.is_empty() { "everyone" } else { who };
        Ok(format!("👋 {} says hello to {who}!", ctx.me))
    }

    /// The panel: drawn when it's opened, and again after every click.
    fn panel(ctx: &mut Ctx) -> Panel {
        let clicks = ctx.store["clicks"].as_u64().unwrap_or(0);
        Panel::new()
            .heading("Hello")
            .text(format!("You've clicked {clicks} time{}.", if clicks == 1 { "" } else { "s" }))
            .row(vec![
                Widget::Button { id: "click".into(), label: "Click me".into() },
                Widget::Button { id: "wave".into(), label: "Wave in chat".into() },
            ])
    }

    /// A click in the panel. Returning `Some(text)` sends it as the person's message.
    fn event(ctx: &mut Ctx, id: &str, _value: &str) -> Option<String> {
        match id {
            "click" => {
                let clicks = ctx.store["clicks"].as_u64().unwrap_or(0) + 1;
                ctx.store = json!({ "clicks": clicks });
                None
            }
            "wave" => Some(format!("👋 {} waves", ctx.me)),
            _ => None,
        }
    }
}

export!(Hello);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_hello() {
        let mut ctx = Ctx { me: "Rae".into(), ..Default::default() };
        assert_eq!(Hello::command(&mut ctx, "hello", "Mara").unwrap(), "👋 Rae says hello to Mara!");
        Hello::event(&mut ctx, "click", "");
        assert_eq!(ctx.store["clicks"], 1);
    }
}
