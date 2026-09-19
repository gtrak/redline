"""gears — a tiny transmission kit."""

from gears.engine import MAX_TORQUE, spin
from gears.render import render

__version__ = "0.3.1"


def hello():
    """Greet the operator."""
    return "gears engaged"


class Gearbox:
    """A gearbox with a fixed ratio table."""

    def __init__(self, ratio):
        self.ratio = ratio

    def apply(self, input_rpm):
        return input_rpm * self.ratio
