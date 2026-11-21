"""Tests of the LCOV filter for the coverage report (`utils/lcov_filter.py`).

Run them with `python3 -m pytest utils`.
"""

from __future__ import annotations

from pathlib import Path
import sys

import pytest

sys.path.insert(0, str(Path(__file__).parent))
import lcov_filter  # noqa: E402

BINDING = """\
use pyo3::prelude::*;

#[pyclass(name = "Thing", module = "ga4gh_sdk")]
pub struct PyThing;

#[pymethods]
impl PyThing {
    fn value(&self) -> u8 {
        1
    }
}
"""

LIBRARY = """\
// #[pymethods] in a comment of another crate
pub fn value() -> u8 {
    1
}
"""


@pytest.fixture
def root(tmp_path: Path) -> Path:
    """A repository with one binding and one library source file.

    Args:
        tmp_path: pytest's temporary directory.

    Returns:
        The repository root.
    """
    (tmp_path / "python/src").mkdir(parents=True)
    (tmp_path / "python/src/thing.rs").write_text(BINDING)
    (tmp_path / "lib/src").mkdir(parents=True)
    (tmp_path / "lib/src/value.rs").write_text(LIBRARY)
    return tmp_path


def _record(path: Path, lines: dict[int, int]) -> str:
    """Build the LCOV record of one source file.

    Args:
        path: Absolute path of the source file.
        lines: Hit count per line number.

    Returns:
        The record, from `SF:` to `end_of_record`.
    """
    hit = sum(1 for count in lines.values() if count)
    data = "".join(f"DA:{line},{count}\n" for line, count in lines.items())
    return (
        f"SF:{path}\nFN:8,value\nFNDA:1,value\nFNF:1\nFNH:1\n"
        f"{data}LF:{len(lines)}\nLH:{hit}\nend_of_record\n"
    )


def test_drops_pyo3_attribute_lines_of_bindings(root: Path) -> None:
    """Only the `#[pyclass]` and `#[pymethods]` lines of the bindings go.

    Args:
        root: The repository root.
    """
    binding = root / "python/src/thing.rs"
    library = root / "lib/src/value.rs"
    report = _record(binding, {3: 0, 6: 0, 8: 1, 9: 1}) + _record(library, {1: 0, 2: 1, 3: 1})

    filtered, dropped = lcov_filter.filter_report(report, root)

    assert dropped == 2
    assert filtered == _record(binding, {8: 1, 9: 1}) + _record(library, {1: 0, 2: 1, 3: 1})


def test_drops_functions_of_attribute_lines(root: Path) -> None:
    """The trampolines' function records go with their lines.

    Args:
        root: The repository root.
    """
    report = (
        "SF:python/src/thing.rs\n"
        "FN:6,trampoline\nFN:8,value\nFNDA:0,trampoline\nFNDA:3,value\nFNF:2\nFNH:1\n"
        "DA:6,0\nDA:8,3\nLF:2\nLH:1\nend_of_record\n"
    )

    filtered, dropped = lcov_filter.filter_report(report, root)

    assert dropped == 1
    assert filtered == (
        "SF:python/src/thing.rs\n"
        "FN:8,value\nFNDA:3,value\nFNF:1\nFNH:1\n"
        "DA:8,3\nLF:1\nLH:1\nend_of_record\n"
    )


def test_keeps_records_without_attribute_lines(root: Path) -> None:
    """A report without PyO3 attribute lines stays unchanged.

    Args:
        root: The repository root.
    """
    report = _record(root / "lib/src/value.rs", {2: 1, 3: 0})

    assert lcov_filter.filter_report(report, root) == (report, 0)


def test_summarizes_lines_per_directory(root: Path) -> None:
    """The summary counts `DA` records per top-level source directory.

    Args:
        root: The repository root.
    """
    report = _record(root / "python/src/thing.rs", {8: 1, 9: 0}) + _record(
        root / "lib/src/value.rs", {2: 1, 3: 1}
    )

    assert lcov_filter.summarize(report, root) == (
        "Directory      Lines   Missed  Covered\n"
        "lib/src            2        0  100.00%\n"
        "python/src         2        1   50.00%\n"
        "TOTAL              4        1   75.00%\n"
    )


@pytest.mark.parametrize(
    "attribute",
    [
        "#[pymethods]",
        "#[pyclass]",
        "  #[pyclass(eq, eq_int)]",
        '#[pyclass(name = "A")]  ',
        "#[pyclass(",
        '#[pyclass(name = "A",',
    ],
)
def test_matches_attribute_forms(attribute: str) -> None:
    """Attribute lines match with arguments and surrounding whitespace.

    Args:
        attribute: A line with a PyO3 attribute.
    """
    assert lcov_filter.ATTRIBUTE.fullmatch(attribute)


@pytest.mark.parametrize(
    "line",
    [
        "#[pyfunction]",
        "#[pyo3(get)]",
        "#[pyclassmethod]",
        "// #[pymethods]",
        "#[pymethods] impl A {}",
    ],
)
def test_ignores_other_lines(line: str) -> None:
    """Other attributes and lines with more than the attribute stay.

    Args:
        line: A source line that holds no lone PyO3 class attribute.
    """
    assert not lcov_filter.ATTRIBUTE.fullmatch(line)


def test_accepts_relative_paths_and_checksums(root: Path) -> None:
    """Relative `SF:` paths and `DA` records with a checksum are handled.

    Args:
        root: The repository root.
    """
    report = "SF:python/src/thing.rs\nDA:6,0,abc\nDA:8,1,def\nLF:2\nLH:1\nend_of_record\n"

    filtered, dropped = lcov_filter.filter_report(report, root)

    assert dropped == 1
    assert filtered == "SF:python/src/thing.rs\nDA:8,1,def\nLF:1\nLH:1\nend_of_record\n"


def test_fails_on_missing_binding_source(root: Path) -> None:
    """A report that names a missing binding source doesn't match the checkout.

    Args:
        root: The repository root.
    """
    with pytest.raises(FileNotFoundError):
        lcov_filter.filter_report(_record(root / "python/src/gone.rs", {1: 0}), root)


def test_counts_files_outside_the_root_separately(
    root: Path, tmp_path_factory: pytest.TempPathFactory
) -> None:
    """Sources outside the repository are neither filtered nor mixed in.

    Args:
        root: The repository root.
        tmp_path_factory: pytest's factory of temporary directories.
    """
    outside = tmp_path_factory.mktemp("registry") / "python/src/thing.rs"
    outside.parent.mkdir(parents=True)
    outside.write_text(BINDING)
    report = _record(outside, {6: 0, 8: 1})

    assert lcov_filter.filter_report(report, root) == (report, 0)
    assert lcov_filter.summarize(report, root).splitlines()[1] == (
        "(outside)          2        1   50.00%"
    )


def test_summarizes_an_empty_report(root: Path) -> None:
    """A report without lines counts as fully covered.

    Args:
        root: The repository root.
    """
    assert lcov_filter.summarize("", root) == (
        "Directory      Lines   Missed  Covered\n"
        "TOTAL              0        0  100.00%\n"
    )


def test_main_defaults_to_the_working_directory(
    root: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    """Without `--root`, the report's paths belong to the working directory.

    Args:
        root: The repository root.
        monkeypatch: pytest's monkeypatch fixture.
        capsys: pytest's output capture fixture.
    """
    monkeypatch.chdir(root)
    (root / "lcov.info").write_text(_record(Path("python/src/thing.rs"), {3: 0, 8: 1}))

    assert lcov_filter.main(["lcov.info"]) == 0

    assert (root / "lcov.info").read_text() == _record(Path("python/src/thing.rs"), {8: 1})
    assert capsys.readouterr().out.startswith("Dropped 1 PyO3 attribute lines\n")


def test_main_writes_report_and_summary(
    root: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """The command line filters a file in place and prints the summary.

    Args:
        root: The repository root.
        capsys: pytest's output capture fixture.
    """
    lcov = root / "lcov.info"
    lcov.write_text(_record(root / "python/src/thing.rs", {6: 0, 8: 1}))

    assert lcov_filter.main([str(lcov), "--root", str(root)]) == 0

    assert lcov.read_text() == _record(root / "python/src/thing.rs", {8: 1})
    output = capsys.readouterr().out
    assert output.startswith("Dropped 1 PyO3 attribute lines\n")
    assert output.endswith("TOTAL              1        0  100.00%\n")
