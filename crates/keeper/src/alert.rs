//! Telling Firelit Games when the Keeper can't do its job by itself: its Claude API credits
//! ran out, or its API key stopped working. It says so once (until it works again) as a
//! report in the server's bug reports, where the owner's Pyr shows a red badge.

use std::path::Path;

use crate::store::Store;

/// What's wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// The Claude API says the credit balance is too low.
    Credits,
    /// The Claude API turned the key down (expired, deleted, or not allowed).
    Key,
}

impl Trouble {
    fn tag(self) -> &'static str {
        match self {
            Trouble::Credits => "credits",
            Trouble::Key => "key",
        }
    }

    fn report(self) -> (&'static str, &'static str) {
        match self {
            Trouble::Credits => (
                "Library Keeper: the Claude API credits ran out",
                "The Library Keeper's Claude API credits have run out, so new versions of Library plugins and skins wait \
                 instead of being checked (nothing goes up unchecked).\n\nAdd credits at console.anthropic.com \
                 (Settings, Billing). The Keeper carries on by itself within a minute: nothing to restart.",
            ),
            Trouble::Key => (
                "Library Keeper: the Claude API turned its key down",
                "The Claude API turned down the Library Keeper's API key (it may have expired or been deleted), so new \
                 versions of Library plugins and skins wait instead of being checked (nothing goes up unchecked).\n\n\
                 Make a new key at console.anthropic.com (Settings, API Keys), then on the VPS run:\n\
                 rm -f /etc/pyr-keeper/api-key && bash /root/pyr-keeper-setup/setup-keeper.sh\n\
                 and paste the new key when it asks.",
            ),
        }
    }
}

/// What a failed review or model lookup means for the owner, if anything (the API's own
/// wording: "Your credit balance is too low…", 401 for a bad key, 403 for one not allowed).
pub fn trouble(e: &anyhow::Error) -> Option<Trouble> {
    let m = format!("{e:#}").to_lowercase();
    if !m.contains("the claude api said") {
        return None;
    }
    if m.contains("credit balance") || m.contains("402") {
        Some(Trouble::Credits)
    } else if m.contains("401") || m.contains("403") || m.contains("authentication") || m.contains("api key") {
        Some(Trouble::Key)
    } else {
        None
    }
}

fn marker(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join("alerted")
}

/// Says so in the bug reports, once for each new trouble.
pub fn raise(store: &Store, state_dir: &Path, t: Trouble) {
    let path = marker(state_dir);
    if std::fs::read_to_string(&path).is_ok_and(|s| s.trim() == t.tag()) {
        return;
    }
    let (title, body) = t.report();
    match store.add_report(title, body) {
        Ok(()) => {
            tracing::warn!("{title}: said so in the bug reports");
            let _ = std::fs::write(&path, t.tag());
        }
        Err(e) => tracing::warn!("{title}, and it couldn't be said in the bug reports: {e:#}"),
    }
}

/// Working again: the next trouble is said again.
pub fn clear(state_dir: &Path) {
    let _ = std::fs::remove_file(marker(state_dir));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_apis_answers_are_told_apart() {
        let e = |m: &str| anyhow::anyhow!("{m}");
        assert_eq!(
            trouble(&e("the Claude API said 400 Bad Request: Your credit balance is too low to access the Anthropic API.")),
            Some(Trouble::Credits)
        );
        assert_eq!(trouble(&e("the Claude API said 401 Unauthorized: invalid x-api-key")), Some(Trouble::Key));
        assert_eq!(trouble(&e("the Claude API said 529: Overloaded")), None, "busy: tried again, not an alert");
        assert_eq!(trouble(&e("connection refused")), None);
    }
}
