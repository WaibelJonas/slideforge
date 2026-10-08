from pathlib import Path

import pytest

# tests/fixtures/CMU-1-Small-Region.svs lives at the repo root, shared with
# the Rust test suite -- see tests/fixtures/README.md for provenance.
FIXTURE_PATH = (
    Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "CMU-1-Small-Region.svs"
)


@pytest.fixture
def fixture_path() -> str:
    assert FIXTURE_PATH.exists(), f"missing test fixture: {FIXTURE_PATH}"
    return str(FIXTURE_PATH)
