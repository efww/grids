"""Setting item builders shared by the cable (STEP 6) and JB (STEP 7) screens."""


def _sel(id, name, options, default, help="", on=None):
    """on: the option a toggle switch means when it is ON (two-option items shown as switches)."""
    d = {"id": id, "name": name, "type": "select", "options": options, "default": default, "help": help}
    if on:
        d["on"] = on
    return d


def _num(id, name, default, unit="", help="", step=1, pct=False):
    return {"id": id, "name": name, "type": "number", "default": default, "unit": unit, "help": help, "step": step,
            "pct": pct}


def _txt(id, name, default, help=""):
    return {"id": id, "name": name, "type": "text", "default": default, "help": help}
