#!/bin/sh
# Pyr for your home folder, on any Linux (Arch and Omarchy, Fedora, Ubuntu, Mint, Debian...):
#
#   curl -fsSL https://pyr.gg/install.sh | sh
#
# Puts Pyr in ~/.local/share/pyr/app/<version>/ (nothing outside your home folder, so no
# password), adds it to your apps menu (with pyr:// invite links, and `pyr` in a terminal) and
# opens it. From then on Pyr updates itself the way it does on Windows: Update, and it's done.
# Running this again puts the newest Pyr in over what's there. Your settings and identity are
# elsewhere (~/.config/pyr) and never touched.
#
# (It all runs from main, on the last line, so a download cut off half way runs nothing.)

main() {
    set -eu
    FROM=${PYR_INSTALL_FROM:-https://pyr.gg}
    NAME=pyr-linux-x86_64.tar.gz

    [ "$(uname -s)" = Linux ] || fail "this is Pyr for Linux. For Windows, get it at https://pyr.gg"
    case "$(uname -m)" in
        x86_64 | amd64) ;;
        *) fail "Pyr for Linux runs on 64-bit Intel and AMD PCs (x86_64) so far, and this one is $(uname -m)." ;;
    esac
    [ "$(id -u)" != 0 ] || fail "run this as yourself, without sudo: Pyr goes in your own home folder."
    case "${HOME:-}" in
        /*) ;;
        *) fail "there's no home folder (\$HOME) to put Pyr in." ;;
    esac
    for tool in tar gzip sha256sum mktemp sed; do
        command -v "$tool" > /dev/null 2>&1 || fail "this needs $tool, and this PC doesn't have it."
    done
    case "$FROM" in
        https://*) PROTO='=https' ;;
        *) PROTO='=http,https' ;;
    esac
    if command -v curl > /dev/null 2>&1; then
        DOWNLOADER=curl
    elif command -v wget > /dev/null 2>&1; then
        DOWNLOADER=wget
    else
        fail "this needs curl or wget to download Pyr."
    fi

    # Where Pyr in your home folder goes (as Pyr itself works it out: home_install.rs).
    case "${XDG_DATA_HOME:-}" in
        /*) DATA=$XDG_DATA_HOME ;;
        *) DATA=$HOME/.local/share ;;
    esac
    ROOT=$DATA/pyr/app
    mkdir -p "$ROOT"
    # (Beside where it goes, so it's moved there, not copied. Pyr's own updates tidy away one
    # left by an install that was stopped.)
    WORK=$(mktemp -d "$ROOT/.incoming-install-XXXXXX")
    trap 'rm -rf "${WORK:?}"' EXIT
    trap 'exit 130' INT TERM HUP

    say "Downloading Pyr..."
    get quiet "$FROM/$NAME.sha256" "$WORK/sum" || fail "couldn't get Pyr from $FROM (is the internet on?)."
    WANT=$(sed -n '1s/^\([0-9a-f]*\).*/\1/p' "$WORK/sum")
    [ ${#WANT} -eq 64 ] || fail "$FROM didn't say what the download should be. Try again in a minute."
    get bar "$FROM/$NAME" "$WORK/$NAME" || fail "the download broke off. Run this again to try once more."
    GOT=$(sha256sum "$WORK/$NAME" | sed 's/ .*//')
    # (Right as the website gets a new version, its download and checksum can be a minute apart.)
    [ "$GOT" = "$WANT" ] || fail "the download didn't come through whole. Run this again in a minute."

    mkdir "$WORK/pyr"
    tar -xzf "$WORK/$NAME" -C "$WORK/pyr" || fail "couldn't unpack the download (is the disk full?)."
    rm -f "$WORK/$NAME"
    VERSION=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$WORK/pyr/pyr-home.json" 2> /dev/null | head -n 1)
    case "$VERSION" in
        "" | .* | *[!0-9A-Za-z.-]*) fail "the download isn't Pyr for your home folder." ;;
    esac
    [ -f "$WORK/pyr/pyr" ] && [ -x "$WORK/pyr/pyr" ] || fail "the download has no Pyr program in it."
    needs "$WORK/pyr/pyr"

    # In place: the version's folder, then `current` pointing at it.
    PREV=$(readlink "$ROOT/current" 2> /dev/null || true)
    if [ -e "$ROOT/$VERSION" ] || [ -L "$ROOT/$VERSION" ]; then
        mv "$ROOT/$VERSION" "$WORK/was"
    fi
    mv "$WORK/pyr" "$ROOT/$VERSION"
    point "$VERSION"
    # Started once to put it in your apps menu (and the rest). That it runs at all says Pyr can
    # start on this PC; if it can't, what was there before is put back.
    if ! "$ROOT/current/pyr" --integrate < /dev/null; then
        rm -rf "${ROOT:?}/${VERSION:?}"
        if [ -d "$WORK/was" ]; then
            mv "$WORK/was" "$ROOT/$VERSION"
        fi
        if [ -n "$PREV" ] && [ -d "$ROOT/$PREV" ]; then
            point "$PREV"
        else
            rm -f "$ROOT/current"
        fi
        fail "Pyr $VERSION couldn't start on this PC (what it said is just above)."
    fi

    say ""
    say "Pyr $VERSION is in your home folder ($ROOT/$VERSION),"
    say "and in your apps menu. It updates itself from now on: no password, nothing to do."
    others
    case ":${PATH:-}:" in
        *":$HOME/.local/bin:"*) ;;
        *) say "(To start it from a terminal as pyr, add ~/.local/bin to your PATH. Or run $ROOT/current/pyr.)" ;;
    esac
    reopen
}

say() {
    printf '%s\n' "$*"
}

fail() {
    printf 'Pyr: %s\n' "$*" >&2
    exit 1
}

# get quiet|bar URL FILE
get() {
    if [ "$DOWNLOADER" = curl ]; then
        if [ "$1" = bar ]; then
            curl -fL --proto "$PROTO" --proto-redir "$PROTO" --retry 3 --progress-bar -o "$3" "$2"
        else
            curl -fsSL --proto "$PROTO" --proto-redir "$PROTO" --retry 3 -o "$3" "$2"
        fi
    else
        if [ "$1" = bar ] && wget --help 2>&1 | grep -q -- --show-progress; then
            wget -q --show-progress -O "$3" "$2"
        else
            wget -q -O "$3" "$2"
        fi
    fi
}

# `current` -> VERSION, in one step (a new link moved over the old one).
point() {
    ln -s "$1" "$WORK/current"
    mv -Tf "$WORK/current" "$ROOT/current" 2> /dev/null || {
        rm -f "$WORK/current"
        ln -sfn "$1" "$ROOT/current"
    }
}

# The libraries Pyr needs that this PC doesn't have, and how to get them.
needs() {
    command -v ldd > /dev/null 2>&1 || return 0
    MISSING=$(ldd "$1" 2> /dev/null | sed -n 's/^[[:space:]]*\([^[:space:]]*\) => not found.*/\1/p' | sort -u)
    [ -n "$MISSING" ] || return 0
    PKGS=""
    for lib in $MISSING; do
        PKGS="$PKGS $(package "$lib")"
    done
    say "Pyr needs these, and this PC doesn't have them yet: $(echo $MISSING)" >&2
    if command -v pacman > /dev/null 2>&1; then
        say "Put them in with:  sudo pacman -S --needed$PKGS" >&2
    elif command -v dnf > /dev/null 2>&1; then
        say "Put them in with:  sudo dnf install$PKGS" >&2
    elif command -v zypper > /dev/null 2>&1; then
        say "Put them in with:  sudo zypper install$PKGS" >&2
    elif command -v apt-get > /dev/null 2>&1; then
        say "Put them in with:  sudo apt install$PKGS" >&2
    fi
    fail "then run this again."
}

# A library's package, by the package manager here.
package() {
    if command -v pacman > /dev/null 2>&1; then
        case "$1" in libasound*) echo alsa-lib ;; libX11*) echo libx11 ;; libstdc++* | libgcc_s*) echo gcc-libs ;; *) echo "$1" ;; esac
    elif command -v dnf > /dev/null 2>&1; then
        case "$1" in libasound*) echo alsa-lib ;; libX11*) echo libX11 ;; libstdc++*) echo libstdc++ ;; libgcc_s*) echo libgcc ;; *) echo "$1" ;; esac
    elif command -v zypper > /dev/null 2>&1; then
        case "$1" in libasound*) echo libasound2 ;; libX11*) echo libX11-6 ;; libstdc++*) echo libstdc++6 ;; libgcc_s*) echo libgcc_s1 ;; *) echo "$1" ;; esac
    else
        case "$1" in libasound*) echo libasound2 ;; libX11*) echo libx11-6 ;; libstdc++*) echo libstdc++6 ;; libgcc_s*) echo libgcc-s1 ;; *) echo "$1" ;; esac
    fi
}

# A Pyr installed for the whole PC as well (a package, or copied there by hand): it stays, and
# the apps menu opens this one now.
others() {
    for p in /usr/bin/pyr /usr/local/bin/pyr; do
        [ -e "$p" ] || continue
        HOW=""
        if command -v pacman > /dev/null 2>&1 && OWNER=$(pacman -Qqo "$p" 2> /dev/null); then
            HOW="sudo pacman -R $OWNER"
        elif command -v dpkg > /dev/null 2>&1 && OWNER=$(dpkg -S "$p" 2> /dev/null | sed -n '1s/:.*//p') && [ -n "$OWNER" ]; then
            HOW="sudo apt remove $OWNER"
        elif command -v rpm > /dev/null 2>&1 && OWNER=$(rpm -qf --qf '%{NAME}\n' "$p" 2> /dev/null); then
            HOW="sudo dnf remove $OWNER"
        fi
        say ""
        if [ -n "$HOW" ]; then
            say "The Pyr installed for the whole PC before ($p) is still there, but it's this one that opens now."
            say "To take the old one away:  $HOW"
        else
            say "There's another Pyr at $p (put there by hand): it's this one that opens now. That one can go:  sudo rm $p"
        fi
    done
}

# The Pyrs of yours running now (any version), by their program.
running() {
    for d in /proc/[0-9]*; do
        EXE=$(readlink "$d/exe" 2> /dev/null) || continue
        case "${EXE##*/}" in
            pyr | "pyr (deleted)" | ingle | "ingle (deleted)") printf '%s\n' "${d#/proc/}" ;;
        esac
    done
}

# Opens the new Pyr: right away if none is open, or once you say so if one is (that one is
# closed first, so it's the new one that opens).
reopen() {
    [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ] || {
        say "Open it from your apps menu."
        return 0
    }
    PIDS=$(running)
    if [ -n "$PIDS" ]; then
        ANSWER=n
        if (exec < /dev/tty) 2> /dev/null; then
            printf 'Pyr is open. Close it and open the new one? (You leave a call you are in.) [Y/n] ' > /dev/tty
            read -r ANSWER < /dev/tty || ANSWER=n
            case "$ANSWER" in "" | [Yy]*) ANSWER=y ;; esac
        fi
        if [ "$ANSWER" != y ]; then
            say "Close Pyr and open it again from your apps menu: that's Pyr $VERSION."
            return 0
        fi
        kill $PIDS 2> /dev/null || true
        i=0
        while [ -n "$(running)" ] && [ $i -lt 20 ]; do
            sleep 0.5
            i=$((i + 1))
        done
        if [ -n "$(running)" ]; then
            say "Pyr didn't close. Close it yourself, then open it from your apps menu: that's Pyr $VERSION."
            return 0
        fi
    fi
    if command -v setsid > /dev/null 2>&1; then
        setsid "$ROOT/current/pyr" > /dev/null 2>&1 < /dev/null &
    else
        nohup "$ROOT/current/pyr" > /dev/null 2>&1 < /dev/null &
    fi
    say "Opening Pyr $VERSION..."
}

main "$@"
