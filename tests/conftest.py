import pytest

from ponder import craft


@pytest.fixture(autouse=True)
def free_clicks(monkeypatch):
    """Tests check currency maths; time costs get their own tests."""
    monkeypatch.setattr(craft, "CHAOS_PER_HOUR", 0.0)
