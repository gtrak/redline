"""Drive the gears demo.

This file carries the USE SITES the probes reference:

- ``import gears.engine`` (plain import: only the dotted name is usable,
  a bare ``engine`` reference must never be hinted)
- ``from gears import *`` (wildcard: origin of the star names is
  opaque to the app — no hint, honest bail)
- ``from gears import engine`` (dotted from-entry: the use site
  ``engine.torque`` is path-shaped, not a bare hint)
- ``from gears.engine import spin as rotate`` (aliased import: the
  bare use site ``rotate`` carries the ORIGINAL import path as its
  scope hint)
"""

import gears.engine
from gears import *
from gears import engine
from gears.engine import spin as rotate

from gears import Gearbox


def main():
    gearbox = Gearbox(3.1)
    rpm = 850
    print(gears.engine.torque(rpm))
    print(rotate(rpm))
    print(engine.torque(rpm))
    print(hello())
    print(gearbox.apply(rpm))


if __name__ == "__main__":
    main()
