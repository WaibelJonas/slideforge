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

## Testing

```sh
pip install -e ".[test]"
pytest
```
