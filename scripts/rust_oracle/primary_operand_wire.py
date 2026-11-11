"""Encode generated primary recipes with explicit runtime operands."""

def encode_primary_operands(source, *, primary, range_, radius, primary_plus_two,
                            radius_or_zero=None, primary_plus_three=None):
    operands = {
        primary: ("primary", {}),
        range_: ("range", {}),
        radius: ("radius", {}),
        primary_plus_two: ("primary_add", {"amount": 2}),
    }
    if radius_or_zero is not None:
        operands[radius_or_zero] = ("radius_or_zero", {})
    if primary_plus_three is not None:
        operands[primary_plus_three] = ("primary_add", {"amount": 3})
    for kind in ("radius", "range"):
        for slot, default in source[f"{kind}_fallbacks"].items():
            slot = int(slot)
            if slot in operands:
                raise ValueError(f"duplicate primary probe value {slot}")
            operands[slot] = (f"{kind}_or_default", {"default": default})

    def encode(value):
        if isinstance(value, dict):
            return {key: encode(child) for key, child in value.items()}
        if isinstance(value, list):
            return [encode(child) for child in value]
        if type(value) is int and value in operands:
            kind, fields = operands[value]
            return {"$operand": {"kind": kind, **fields}}
        return value

    return {
        key: encode(value)
        for key, value in source.items()
        if key not in {"radius_fallbacks", "range_fallbacks"}
    }
