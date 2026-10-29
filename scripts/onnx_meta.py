#!/usr/bin/env python3
"""Write one ONNX custom-metadata value to a file, byte for byte.

    python -I scripts/onnx_meta.py MODEL.onnx KEY OUT_FILE

Used by fetch-models.{sh,ps1} --with-dict: the PP-OCR recognition model embeds
its character dictionary as the metadata entry "character" (one character per
line, PaddleOCR dict.txt format, without the CTC blank / trailing space).

Standard library only. ModelProto.metadata_props is field 14 of the ONNX
protobuf; each entry is a StringStringEntryProto (key = 1, value = 2). Only the
top level of the file is walked, so the (large) graph is skipped, not decoded.
"""
import sys

METADATA_PROPS = 14  # ModelProto.metadata_props


def read_varint(buf, pos):
    shift = result = 0
    while True:
        byte = buf[pos]
        pos += 1
        result |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return result, pos
        shift += 7


def fields(buf, pos=0, end=None):
    """Yield (field_number, wire_type, value) for one protobuf message."""
    end = len(buf) if end is None else end
    while pos < end:
        tag, pos = read_varint(buf, pos)
        number, wire = tag >> 3, tag & 7
        if wire == 0:  # varint
            value, pos = read_varint(buf, pos)
        elif wire == 1:  # fixed64
            value, pos = buf[pos : pos + 8], pos + 8
        elif wire == 2:  # length-delimited
            length, pos = read_varint(buf, pos)
            value, pos = (pos, pos + length), pos + length
        elif wire == 5:  # fixed32
            value, pos = buf[pos : pos + 4], pos + 4
        else:
            raise ValueError(f"unsupported protobuf wire type {wire}")
        yield number, wire, value


def metadata_value(buf, key):
    wanted = key.encode("utf-8")
    for number, wire, span in fields(buf):
        if number != METADATA_PROPS or wire != 2:
            continue
        entry = {n: buf[s:e] for n, w, (s, e) in fields(buf, *span) if w == 2}
        if entry.get(1) == wanted:
            return entry.get(2, b"")
    return None


def main(argv):
    if len(argv) != 4:
        sys.stderr.write(__doc__)
        return 2
    model, key, out = argv[1:]
    with open(model, "rb") as f:
        value = metadata_value(f.read(), key)
    if value is None:
        sys.stderr.write(f"error: metadata key {key!r} not found in {model}\n")
        return 1
    with open(out, "wb") as f:
        f.write(value)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
