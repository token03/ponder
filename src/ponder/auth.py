"""The pathofexile.com session cookie (POESESSID) for trade lookups.

It is a login token for your account, so treat it like a password. Where it
comes from, first match wins:

1. the POESESSID environment variable (e.g. from a git-ignored .env)
2. this process's memory, once you've typed it in (never written anywhere)
3. the system keychain, if you chose to save it there (`ponder login`):
   Windows Credential Manager, macOS Keychain or the Linux Secret Service,
   which encrypt it under your OS login. WSL usually has none; then nothing
   is saved and you're asked again next run.
"""

from __future__ import annotations

import getpass
import os
import sys

import keyring
from keyring.errors import KeyringError

HELP_URL = "https://poedb.tw/us/POESESSID"
SERVICE = "ponder"
ACCOUNT = "POESESSID"

_memory: str | None = None


def get() -> str | None:
    """The cookie if we already have it; never prompts."""
    return os.environ.get("POESESSID") or _memory or _from_keychain()


def remember(value: str, save: bool = False) -> bool:
    """Use `value` for the rest of this run; with `save`, also store it in the keychain.

    Returns whether it was saved.
    """
    global _memory
    _memory = value.strip()
    if save and keychain_available():
        try:
            keyring.set_password(SERVICE, ACCOUNT, _memory)
            return True
        except KeyringError:
            pass
    return False


def forget() -> None:
    """Drop it from memory and the keychain (the environment variable is yours to unset)."""
    global _memory
    _memory = None
    if keychain_available():
        try:
            keyring.delete_password(SERVICE, ACCOUNT)
        except KeyringError:
            pass


def keychain_available() -> bool:
    """True when the OS has a real credential store (not the "fail" or plaintext ones)."""
    try:
        backend = keyring.get_keyring()
    except Exception:  # a broken backend shouldn't break the app
        return False
    return getattr(backend, "priority", 0) >= 1 and "fail" not in type(backend).__module__ and "plaintext" not in type(backend).__module__.lower()


def source() -> str:
    """Where the cookie in use comes from, for showing the user (never the value)."""
    if os.environ.get("POESESSID"):
        return "environment variable"
    if _memory:
        return "typed in this session (not saved)"
    if _from_keychain():
        return "system keychain"
    return "not set"


def prompt(offer_save: bool = True) -> str | None:
    """Ask on the terminal with hidden input. Returns None when there's no terminal or no answer."""
    if not sys.stdin.isatty():
        return None
    say = sys.stderr  # stdout may be JSON going to another program
    print("Trade lookups need your pathofexile.com session cookie (POESESSID).", file=say)
    print(f"How to find it: {HELP_URL}", file=say)
    print("It is a login token: don't share it. Input is hidden; press Enter to skip.", file=say)
    value = getpass.getpass("POESESSID: ", stream=say).strip()
    if not value:
        return None
    save = False
    if offer_save and keychain_available():
        say.write("Save it in your system keychain for next time? [y/N] ")
        say.flush()
        save = sys.stdin.readline().strip().lower() == "y"
    saved = remember(value, save)
    print("Saved in your system keychain." if saved else "Using it for this run only; nothing was saved.", file=say)
    return value


def _from_keychain() -> str | None:
    if not keychain_available():
        return None
    try:
        return keyring.get_password(SERVICE, ACCOUNT)
    except KeyringError:
        return None
