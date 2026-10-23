import pytest

from ponder import auth


@pytest.fixture(autouse=True)
def clean(monkeypatch):
    monkeypatch.delenv("POESESSID", raising=False)
    monkeypatch.setattr(auth, "_memory", None)
    monkeypatch.setattr(auth, "keychain_available", lambda: False)


def test_typed_cookie_lives_in_memory_only():
    assert auth.get() is None and auth.source() == "not set"
    assert auth.remember(" abc ", save=True) is False  # no keychain: nothing saved
    assert auth.get() == "abc" and auth.source().startswith("typed")
    auth.forget()
    assert auth.get() is None


def test_environment_wins(monkeypatch):
    auth.remember("typed")
    monkeypatch.setenv("POESESSID", "from-env")
    assert auth.get() == "from-env" and auth.source() == "environment variable"


def test_keychain_used_when_available(monkeypatch):
    store = {}
    monkeypatch.setattr(auth, "keychain_available", lambda: True)
    monkeypatch.setattr(auth.keyring, "set_password", lambda s, a, v: store.__setitem__((s, a), v))
    monkeypatch.setattr(auth.keyring, "get_password", lambda s, a: store.get((s, a)))
    assert auth.remember("saved-one", save=True) is True
    monkeypatch.setattr(auth, "_memory", None)  # a later run
    assert auth.get() == "saved-one" and auth.source() == "system keychain"


def test_no_prompt_without_a_terminal(monkeypatch):
    monkeypatch.setattr(auth.sys.stdin, "isatty", lambda: False, raising=False)
    assert auth.prompt() is None
