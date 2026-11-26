"""Instrument Type text from ISA 5.1 function letters (decision 13).

PT -> Pressure Transmitter, PDT -> DP Transmitter, FV -> Flow Control Valve ...
Only drawn instruments get this name; rows created by a typical (limit
switches, motor IO, package IO) keep the typical's own description.
"""
from __future__ import annotations

import re

FIRST = {
    "A": "Analyzer", "B": "Burner", "C": "Conductivity", "D": "Density", "E": "Voltage", "F": "Flow",
    "G": "Gauging", "H": "Hand", "I": "Current", "J": "Power", "K": "Time", "L": "Level", "M": "Moisture",
    "N": "", "O": "", "P": "Pressure", "Q": "Quantity", "R": "Radiation", "S": "Speed", "T": "Temperature",
    "U": "Multivariable", "V": "Vibration", "W": "Weight", "X": "", "Y": "Event", "Z": "Position",
}
WHOLE = {
    "PDT": "DP Transmitter", "PDI": "DP Indicator", "PDG": "DP Gauge", "PDV": "DP Control Valve",
    "PDS": "DP Switch", "FQI": "Flow Totalizer", "FQ": "Flow Totalizer",
    "PSV": "Pressure Safety Valve", "PRV": "Pressure Relief Valve", "PVRV": "Pressure Vacuum Relief Valve",
    "VRV": "Vacuum Relief Valve", "PSE": "Rupture Disk", "TSV": "Thermal Safety Valve",
    "PCV": "Pressure Control Valve (Self-actuated)", "TCV": "Temperature Control Valve (Self-actuated)",
    "LCV": "Level Control Valve (Self-actuated)", "FCV": "Flow Control Valve (Self-actuated)",
    "XV": "On-Off Valve", "SDV": "Shutdown Valve", "BDV": "Blowdown Valve", "UV": "On-Off Valve (Multi-variable)",
    "MOV": "Motor Operated Valve", "HV": "Hand Control Valve",
    "TW": "Thermowell", "SC": "Sample Connection", "RO": "Restriction Orifice", "FO": "Restriction Orifice",
    "FE": "Flow Element", "TE": "Temperature Element", "AE": "Analyzer Element", "LE": "Level Element",
    "FG": "Sight Glass", "LG": "Level Gauge", "PG": "Pressure Gauge", "TG": "Temperature Gauge",
    "HS": "Hand Switch", "EH": "Electric Heater",
    "GD": "Gas Detector", "FD": "Flame Detector", "SD": "Smoke Detector", "HD": "Heat Detector",
    "XA": "Alarm", "XL": "Status Light", "AA": "Alarm (Analyzer)", "AL": "Alarm Light",
}
TAIL = [
    ("T", "Transmitter"), ("IT", "Indicating Transmitter"), ("V", "Control Valve"), ("G", "Gauge"),
    ("E", "Element"), ("I", "Indicator"), ("SHH", "Switch High High"), ("SLL", "Switch Low Low"),
    ("SH", "Switch High"), ("SL", "Switch Low"), ("S", "Switch"), ("Y", "Relay / Converter"),
    ("Z", "Actuator"), ("W", "Well"), ("C", "Controller"),
]


def instrument_type(func: str, fallback: str = "") -> str:
    f = (func or "").upper()
    if f in WHOLE:
        return WHOLE[f]
    if not f or not re.fullmatch(r"[A-Z]{2,5}", f):
        return fallback
    first = FIRST.get(f[0], "")
    rest = f[1:]
    if rest.startswith("D") and len(rest) > 1 and f[0] in "PT":
        first, rest = ("DP" if f[0] == "P" else "Differential " + first), rest[1:]
    for k, name in sorted(TAIL, key=lambda x: -len(x[0])):
        if rest == k:
            return f"{first} {name}".strip() if first else (fallback or name)
    return fallback
