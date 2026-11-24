"""Read-only carving of table rows whose b-tree pages were orphaned by corruption.

The scanner never trusts sqlite_master: it walks every page of the file,
decodes each leaf cell with the SQLite record format and keeps the cells
whose column shape matches a known table. Rows found in index-leaf pages
(these carry the full record but no rowid) are reported with ``seq=None``.
"""
import mmap
import re
import struct

INT_SIZES = {1: 1, 2: 2, 3: 3, 4: 4, 5: 6, 6: 8}
ULID = re.compile(r'^[0-9A-HJKMNP-TV-Z]{26}$')
SESSION_ID = re.compile(r'^(sub-)?[0-9A-HJKMNP-TV-Z]{26}$')
MESSAGE_ROLES = frozenset({'user', 'assistant', 'system', 'tool', 'function'})
TS_MIN, TS_MAX = 1_600_000_000_000, 1_900_000_000_000
TABLE_LEAF, INDEX_LEAF = 13, 10
MAX_PAYLOAD = 64 * 1024 * 1024


def varint(buf, i):
    value = 0
    for k in range(9):
        byte = buf[i]
        i += 1
        if k == 8:
            return (value << 8) | byte, i
        value = (value << 7) | (byte & 0x7f)
        if not byte & 0x80:
            return value, i
    return value, i


def decode_record(payload):
    header_size, i = varint(payload, 0)
    if header_size < 2 or header_size > len(payload):
        raise ValueError('record header')
    types = []
    while i < header_size:
        serial, i = varint(payload, i)
        types.append(serial)
    values = []
    i = header_size
    for serial in types:
        if serial == 0:
            values.append(None)
        elif serial in INT_SIZES:
            n = INT_SIZES[serial]
            raw = payload[i:i + n]
            if len(raw) != n:
                raise ValueError('short integer')
            values.append(int.from_bytes(raw, 'big', signed=True))
            i += n
        elif serial == 7:
            values.append(struct.unpack('>d', payload[i:i + 8])[0])
            i += 8
        elif serial in (8, 9):
            values.append(serial - 8)
        elif serial >= 12:
            n = (serial - 12) // 2 if serial % 2 == 0 else (serial - 13) // 2
            raw = payload[i:i + n]
            if len(raw) != n:
                raise ValueError('short text')
            values.append(raw if serial % 2 == 0 else raw.decode('utf-8', 'replace'))
            i += n
        else:
            raise ValueError('serial type')
    if i != len(payload):
        raise ValueError('trailing bytes')
    return values


def local_size(size, page_size, index_page):
    usable = page_size
    if index_page:
        max_local = (usable - 12) * 64 // 255 - 23
    else:
        max_local = usable - 35
    min_local = (usable - 12) * 32 // 255 - 23
    if size <= max_local:
        return size
    k = min_local + (size - min_local) % (usable - 4)
    return k if k <= max_local else min_local


def read_payload(mm, page_count, page_size, pg, start, size, index_page):
    local = local_size(size, page_size, index_page)
    data = pg[start:start + local]
    if local < size:
        overflow = struct.unpack('>I', pg[start + local:start + local + 4])[0]
        rest = size - local
        parts = [data]
        seen = set()
        while rest > 0:
            if not overflow or overflow > page_count or overflow in seen:
                raise ValueError('broken overflow chain')
            seen.add(overflow)
            op = mm[(overflow - 1) * page_size:overflow * page_size]
            overflow = struct.unpack('>I', op[0:4])[0]
            take = min(rest, page_size - 4)
            parts.append(op[4:4 + take])
            rest -= take
        data = b''.join(parts)
    return data


def is_session_id(value):
    return isinstance(value, str) and SESSION_ID.match(value) is not None


def is_message(values):
    """messages(seq, id, session_id, role, agent, model, blocks_json, provider_state_json,
    usage_json, created_at, synthetic, display, mode, summary)."""
    if len(values) != 14 or values[0] is not None:
        return False
    _, mid, sid, role, _, _, blocks, _, usage, created, synthetic, _, _, summary = values
    if not (isinstance(mid, str) and ULID.match(mid)):
        return False
    if not is_session_id(sid):
        return False
    if not (isinstance(role, str) and role in MESSAGE_ROLES):
        return False
    if not (isinstance(blocks, str) and blocks.startswith('[')):
        return False
    if not (isinstance(usage, str) and usage.startswith('{')):
        return False
    if not (isinstance(created, int) and TS_MIN <= created <= TS_MAX):
        return False
    return synthetic in (0, 1) and summary in (0, 1)


def is_event(values):
    """session_events(seq, session_id, type, payload_json, sse_kind, ts)."""
    if len(values) != 6 or values[0] is not None:
        return False
    _, sid, kind, payload, _, ts = values
    if not is_session_id(sid):
        return False
    if not (isinstance(kind, str) and kind and kind.isprintable()):
        return False
    if not (isinstance(payload, str) and payload.startswith('{')):
        return False
    return isinstance(ts, int) and TS_MIN <= ts <= TS_MAX


def leaf_cells(mm, page_count, page_size, page_no, index_page):
    """Yield (seq_or_None, values) for each decodable cell of one leaf page."""
    pg = mm[(page_no - 1) * page_size:page_no * page_size]
    count = struct.unpack('>H', pg[3:5])[0]
    if count == 0 or count > page_size // 4:
        return
    for i in range(count):
        pointer = struct.unpack('>H', pg[8 + 2 * i:10 + 2 * i])[0]
        if pointer < 8 or pointer >= page_size:
            continue
        try:
            size, j = varint(pg, pointer)
            seq = None
            if not index_page:
                seq, j = varint(pg, j)
            if size < 2 or size > MAX_PAYLOAD or j >= page_size:
                continue
            yield seq, decode_record(read_payload(mm, page_count, page_size, pg, j, size, index_page))
        except (ValueError, IndexError, struct.error):
            continue


def page_size_of(header):
    raw = struct.unpack('>H', header[16:18])[0]
    return 65536 if raw == 1 else raw


def carve(path, accept):
    """Yield (page_no, seq, values) for every leaf cell whose record satisfies ``accept``.

    Table-leaf pages are read with the rowid layout; index-leaf pages with the
    key-only layout (``seq`` is None there because the rowid is not stored).
    """
    with open(path, 'rb') as handle, mmap.mmap(handle.fileno(), 0, access=mmap.ACCESS_READ) as mm:
        page_size = page_size_of(mm[:100])
        page_count = len(mm) // page_size
        for page_no in range(2, page_count + 1):
            kind = mm[(page_no - 1) * page_size:(page_no - 1) * page_size + 1][0]
            if kind == TABLE_LEAF:
                cells = leaf_cells(mm, page_count, page_size, page_no, False)
            elif kind == INDEX_LEAF:
                cells = leaf_cells(mm, page_count, page_size, page_no, True)
            else:
                continue
            for seq, values in cells:
                if accept(values):
                    yield page_no, seq, values


def is_message_or_event(values):
    return is_message(values) or is_event(values)
