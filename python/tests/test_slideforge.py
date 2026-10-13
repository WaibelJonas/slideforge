import numpy as np
import pytest

import slideforge


def test_metadata_reads_real_fixture(fixture_path):
    mpp, objective_power, levels = slideforge.metadata(fixture_path)

    assert mpp == pytest.approx(0.499)
    assert objective_power == pytest.approx(20.0)
    assert levels == [(2220, 2967, 1.0)]


def test_metadata_missing_file_raises_file_not_found():
    with pytest.raises(FileNotFoundError):
        slideforge.metadata("does_not_exist.svs")


def test_extract_requires_exactly_one_of_level_or_target_mpp(fixture_path):
    with pytest.raises(ValueError):
        slideforge.extract(fixture_path)

    with pytest.raises(ValueError):
        slideforge.extract(fixture_path, level=0, target_mpp=0.5)


def test_extract_rejects_out_of_bounds_level(fixture_path):
    with pytest.raises(IndexError):
        slideforge.extract(fixture_path, level=99)


def test_extract_writes_tfrecord_and_returns_tile_counts(fixture_path, tmp_path):
    tfrecord_path = tmp_path / "out.tfrecord"

    kept, total = slideforge.extract(
        fixture_path,
        tfrecord=str(tfrecord_path),
        level=0,
        min_tissue_fraction=0.1,
        stain_normalize=True,
    )

    assert total == 130
    assert 0 < kept < total
    assert tfrecord_path.stat().st_size > 0


def test_extract_without_tfrecord_or_tile_dir_still_counts(fixture_path):
    # No output destinations given -- just the tissue-filter counts.
    kept, total = slideforge.extract(fixture_path, level=0, min_tissue_fraction=0.1)
    assert total == 130
    assert 0 < kept < total


def test_extract_arrays_returns_matching_shapes_and_dtype(fixture_path):
    tiles = slideforge.extract_arrays(
        fixture_path,
        level=0,
        min_tissue_fraction=0.1,
    )

    assert len(tiles) > 0
    for tile_x, tile_y, array in tiles:
        assert isinstance(tile_x, int)
        assert isinstance(tile_y, int)
        assert array.shape == (240, 240, 3)
        assert array.dtype == np.uint8


def test_extract_and_extract_arrays_agree_on_kept_count(fixture_path):
    kept, _total = slideforge.extract(fixture_path, level=0, min_tissue_fraction=0.1)
    tiles = slideforge.extract_arrays(fixture_path, level=0, min_tissue_fraction=0.1)

    assert len(tiles) == kept


# Grid mode: window size == native tile size (240px @ 0.499 mpp -> 119.76um),
# same case the Rust invariant test uses.
GRID_TILE_UM = 240.0 * 0.499


def test_grid_mode_drops_boundary_windows_native_would_pad(fixture_path):
    native_total = slideforge.extract(fixture_path, level=0)[1]
    grid_total = slideforge.extract(fixture_path, level=0, tile_px=240, tile_um=GRID_TILE_UM)[1]

    assert native_total == 130  # 10x13 native tile grid
    assert grid_total == 108  # 9x12 -- Grid drops the padded boundary row/column


def test_grid_mode_matches_native_pixel_for_pixel_over_their_overlap(fixture_path):
    native = {(x, y): arr for x, y, arr in slideforge.extract_arrays(fixture_path, level=0)}
    grid = slideforge.extract_arrays(fixture_path, level=0, tile_px=240, tile_um=GRID_TILE_UM)

    assert len(grid) == 108
    for x, y, arr in grid:
        np.testing.assert_array_equal(arr, native[(x, y)])


def test_grid_mode_resizes_output_to_tile_px(fixture_path):
    tiles = slideforge.extract_arrays(fixture_path, level=0, tile_px=96, tile_um=GRID_TILE_UM)

    assert len(tiles) == 108
    for _x, _y, arr in tiles:
        assert arr.shape == (96, 96, 3)


@pytest.mark.parametrize(
    "kwargs",
    [
        {"tile_px": 240},
        {"tile_um": GRID_TILE_UM},
        {"stride_um": 50.0},
    ],
)
def test_grid_params_must_be_given_together(fixture_path, kwargs):
    with pytest.raises(ValueError):
        slideforge.extract(fixture_path, level=0, **kwargs)
