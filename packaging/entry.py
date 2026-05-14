"""PyInstaller entry. Imports the package so relative imports inside resolve."""

import sys

from apollo_fleet.tray import main

if __name__ == "__main__":
    sys.exit(main())
