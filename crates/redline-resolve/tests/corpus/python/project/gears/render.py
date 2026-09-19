"""Render gear telemetry into JSON and point at the spool file."""

import json
import os.path

from gears.engine import torque


def render(rpm, spool_dir):
    """Serialize the torque reading and return its spool path."""
    payload = {"rpm": rpm, "torque": torque(rpm)}
    text = json.dumps(payload)
    return os.path.join(spool_dir, "telemetry.json")
