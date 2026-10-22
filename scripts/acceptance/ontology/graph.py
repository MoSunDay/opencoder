"""Verify the 3D graph using an isolated Server and matching embedded assets."""
from pathlib import Path
from catalog import main

if __name__ == "__main__":
    main(Path(__file__).with_suffix(".mjs"))
