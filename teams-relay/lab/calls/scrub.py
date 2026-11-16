"""Removes credentials from a trace JSON in place: JWT-shaped strings anywhere, and the values of authorization,
x-skypetoken and cookie headers. Prints how many were replaced."""
import json, re, sys

JWT = re.compile(r"eyJ[\w-]{10,}(\.[\w-]*){2,4}")
SECRET_HEADERS = {"authorization", "x-skypetoken", "cookie", "set-cookie"}
count = 0


def clean(v):
    global count
    if isinstance(v, dict):
        out = {}
        for k, x in v.items():
            if k.lower() in SECRET_HEADERS and isinstance(x, str):
                out[k] = "<redacted>"
                count += 1
            else:
                out[k] = clean(x)
        return out
    if isinstance(v, list):
        return [clean(x) for x in v]
    if isinstance(v, str):
        new, n = JWT.subn("<redacted-jwt>", v)
        count += n
        return new
    return v


for path in sys.argv[1:]:
    data = json.load(open(path, encoding="utf-8"))
    data = clean(data)
    json.dump(data, open(path, "w", encoding="utf-8"))
print("replaced", count)
