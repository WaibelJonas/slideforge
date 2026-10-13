# slideforge (Python bindings)

Python bindings for [slideforge](https://github.com/WaibelJonas/slideforge), a Rust library for
whole-slide image (WSI) tile extraction with tissue detection and stain normalization.

## Install (development)

```sh
cd python
pip install maturin
maturin develop --release
```

## Usage

```python
import slideforge

mpp, objective_power, levels = slideforge.metadata("slide.svs")

# Extract to a .tfrecord (streams tiles -- safe for gigapixel slides)
kept, total = slideforge.extract(
    "slide.svs",
    tfrecord="slide.tfrecord",
    target_mpp=0.5,
    min_tissue_fraction=0.1,
    stain_normalize=True,
)

# Or collect tiles directly into memory as numpy arrays (small slides/testing only)
tiles = slideforge.extract_arrays(
    "slide.svs",
    level=0,
    min_tissue_fraction=0.1,
)
for tile_x, tile_y, array in tiles:
    ...  # array.shape == (height, width, 3), dtype uint8
```

`extract` and `extract_arrays` each require exactly one of `level` (a specific pyramid level
index) or `target_mpp` (a target microns-per-pixel resolution).

### Grid mode

By default tiles come from the slide's native tile grid. Passing `tile_px` and `tile_um` together
switches to fixed-size windows on a regular grid defined in physical units, independent of the
native tile grid -- matching the tiling convention used by tools like slideflow:

```python
tiles = slideforge.extract_arrays(
    "slide.svs",
    level=0,
    tile_px=256,       # output tile size in pixels
    tile_um=128.0,     # window size in microns, read at native resolution then resized to tile_px
    stride_um=128.0,   # distance between window origins; omit for non-overlapping windows
)
```

`tile_px`/`tile_um` must be given together (or not at all); `stride_um` additionally requires
both. One behavior difference from native mode worth knowing: a trailing window that would extend
past the slide's edge is **dropped**, not padded like a native boundary tile is -- so grid mode
can yield fewer tiles than native mode at an otherwise-equivalent size.

## Testing

```sh
pip install -e ".[test]"
pytest
```
