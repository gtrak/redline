"""Engine primitives: torque curves and spin math."""

MAX_TORQUE = 320.0


def spin(rpm):
    """Return true when the engine is actually turning."""
    return rpm > 0


def torque(rpm):
    """Torque curve: ramps up, then flattens."""
    if rpm <= 0:
        return 0.0
    return min(MAX_TORQUE, rpm / 10.0)


class Motor:
    """A motor with a nameplate rating."""

    def __init__(self, rating_kw):
        self.rating_kw = rating_kw

    def power(self, rpm):
        return torque(rpm) * rpm / 9550.0
