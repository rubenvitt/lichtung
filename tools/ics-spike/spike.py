#!/usr/bin/env python3
"""ICS feed spike for Lichtung (LIC-14, spike S3).

Fetches an ICS feed (e.g. the published Outlook/Exchange work calendar)
twice and prints anonymized statistics: how many events, series, overrides,
which time zones and X-properties, the BUSYSTATUS distribution, the date
range, the size, and whether the server supports conditional requests
(ETag / Last-Modified -> 304). It never prints titles, descriptions,
locations, people, UIDs or the feed URL.

Only the Python standard library is used. The URL is read from the
environment or prompted for (hidden); it is never written anywhere.

    python3 tools/ics-spike/spike.py
    LICHTUNG_SPIKE_ICS_URL='https://…' python3 tools/ics-spike/spike.py --json

See README.md next to this file.
"""

from __future__ import annotations

import argparse
import base64
import collections
import getpass
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

# Same limits as lichtung-server (server/crates/lichtung-server/src/ics_feed.rs).
MAX_FEED_BYTES = 20 * 1024 * 1024
MAX_REDIRECTS = 5
TIMEOUT = 60


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: D401
        return None


OPENER = urllib.request.build_opener(NoRedirect)


def normalize_url(raw: str) -> str:
    raw = raw.strip()
    scheme, sep, rest = raw.partition(":")
    if not sep:
        sys.exit("Das ist keine URL.")
    scheme = scheme.lower()
    if scheme in ("webcal", "webcals"):
        return "https:" + rest
    if scheme not in ("http", "https"):
        sys.exit("Die URL muss mit https://, http://, webcal:// oder webcals:// beginnen.")
    return raw


def fetch(url: str, auth: str | None, headers: dict[str, str]) -> dict:
    """GET with manual redirects. Returns status, headers, body, redirects."""
    redirects = 0
    hosts_changed = False
    start = time.monotonic()
    first_host = urllib.parse.urlsplit(url).hostname
    while True:
        req = urllib.request.Request(url, method="GET")
        req.add_header("User-Agent", "lichtung-ics-spike/0.1")
        req.add_header("Accept", "text/calendar, */*;q=0.5")
        if auth and urllib.parse.urlsplit(url).hostname == first_host:
            req.add_header("Authorization", auth)
        for k, v in headers.items():
            req.add_header(k, v)
        try:
            resp = OPENER.open(req, timeout=TIMEOUT)
            status = resp.status
        except urllib.error.HTTPError as e:
            resp = e
            status = e.code
        except urllib.error.URLError as e:
            sys.exit(f"Keine Verbindung: {type(e.reason).__name__}")
        hdrs = {k.lower(): v for k, v in resp.headers.items()}
        if status in (301, 302, 303, 307, 308):
            redirects += 1
            if redirects > MAX_REDIRECTS:
                sys.exit(f"Mehr als {MAX_REDIRECTS} Weiterleitungen.")
            nxt = urllib.parse.urljoin(url, hdrs.get("location", ""))
            if urllib.parse.urlsplit(nxt).hostname != first_host:
                hosts_changed = True
            url = nxt
            continue
        body = resp.read(MAX_FEED_BYTES + 1) if status == 200 else b""
        return {
            "status": status,
            "headers": hdrs,
            "body": body,
            "redirects": redirects,
            "redirect_host_changed": hosts_changed,
            "seconds": round(time.monotonic() - start, 2),
        }


# ----- parsing (only structure, never content) -----------------------------------------

def unfold(text: str) -> list[str]:
    lines: list[str] = []
    for raw in re.split(r"\r\n|\n|\r", text):
        if raw[:1] in (" ", "\t") and lines:
            lines[-1] += raw[1:]
        else:
            lines.append(raw)
    return [l for l in lines if l.strip()]


def split_line(line: str) -> tuple[str, dict[str, str], str]:
    """NAME;PARAM=V:VALUE (quoted parameter values may contain ':' and ';')."""
    name_end = None
    in_quotes = False
    for i, ch in enumerate(line):
        if ch == '"':
            in_quotes = not in_quotes
        elif ch == ":" and not in_quotes:
            name_end = i
            break
    if name_end is None:
        return line.upper(), {}, ""
    head, value = line[:name_end], line[name_end + 1:]
    parts = re.findall(r'(?:[^;"]|"[^"]*")+', head)
    name = parts[0].upper() if parts else ""
    params = {}
    for p in parts[1:]:
        k, _, v = p.partition("=")
        params[k.upper()] = v.strip('"')
    return name, params, value


def parse(text: str) -> list[dict]:
    """Returns components as dicts: {name, props: [(name, params, value)], children}."""
    root = {"name": "ROOT", "props": [], "children": []}
    stack = [root]
    for line in unfold(text):
        name, params, value = split_line(line)
        if name == "BEGIN":
            comp = {"name": value.strip().upper(), "props": [], "children": []}
            stack[-1]["children"].append(comp)
            stack.append(comp)
        elif name == "END":
            if len(stack) > 1:
                stack.pop()
        else:
            stack[-1]["props"].append((name, params, value))
    return root["children"]


def prop(comp: dict, name: str):
    for n, params, value in comp["props"]:
        if n == name:
            return params, value
    return None


def date_of(value: str) -> str | None:
    m = re.match(r"(\d{4})(\d{2})(\d{2})", value.strip())
    return f"{m.group(1)}-{m.group(2)}" if m else None


def analyze(body: bytes) -> dict:
    text = body.decode("utf-8", "replace")
    stats: dict = {}
    stats["bom"] = text.startswith("﻿")
    text = text.lstrip("﻿")
    stats["line_endings"] = "CRLF" if "\r\n" in text else ("LF" if "\n" in text else "keine")
    raw_lines = re.split(r"\r\n|\n|\r", text)
    stats["folded_lines"] = sum(1 for l in raw_lines if l[:1] in (" ", "\t"))
    stats["longest_line"] = max((len(l.encode()) for l in raw_lines), default=0)
    stats["starts_with_vcalendar"] = text.lstrip().upper().startswith("BEGIN:VCALENDAR")

    top = parse(text)
    cals = [c for c in top if c["name"] == "VCALENDAR"]
    stats["vcalendars"] = len(cals)
    comp_counts = collections.Counter()
    cal_x = collections.Counter()
    ev_x = collections.Counter()
    busy = collections.Counter()
    intended = collections.Counter()
    transp = collections.Counter()
    status = collections.Counter()
    klass = collections.Counter()
    freq = collections.Counter()
    tzid_used = collections.Counter()
    tzid_defined = collections.Counter()
    value_kinds = collections.Counter()
    by_uid: dict[str, list[dict]] = collections.defaultdict(list)
    no_uid = 0
    months: list[str] = []
    header = {}
    for cal in cals:
        for n, _, v in cal["props"]:
            if n.startswith("X-"):
                cal_x[n] += 1
            if n in ("PRODID", "METHOD", "X-WR-TIMEZONE", "X-PUBLISHED-TTL", "REFRESH-INTERVAL"):
                header[n] = v.strip()
        header["X-WR-CALNAME vorhanden"] = "ja" if prop(cal, "X-WR-CALNAME") else "nein"
        header["X-APPLE-CALENDAR-COLOR vorhanden"] = "ja" if prop(cal, "X-APPLE-CALENDAR-COLOR") else "nein"
        for c in cal["children"]:
            comp_counts[c["name"]] += 1
            if c["name"] == "VTIMEZONE":
                t = prop(c, "TZID")
                if t:
                    tzid_defined[t[1].strip()] += 1
            if c["name"] != "VEVENT":
                continue
            for n, params, v in c["props"]:
                if n.startswith("X-"):
                    ev_x[n] += 1
                if "TZID" in params:
                    tzid_used[params["TZID"]] += 1
                if n in ("DTSTART", "DTEND", "RECURRENCE-ID"):
                    if params.get("VALUE", "").upper() == "DATE" or re.fullmatch(r"\d{8}", v.strip()):
                        value_kinds["Datum (ganztägig)"] += 1
                    elif v.strip().upper().endswith("Z"):
                        value_kinds["UTC"] += 1
                    elif "TZID" in params:
                        value_kinds["mit TZID"] += 1
                    else:
                        value_kinds["floating"] += 1
            b = prop(c, "X-MICROSOFT-CDO-BUSYSTATUS")
            busy[b[1].strip().upper() if b else "(fehlt)"] += 1
            i = prop(c, "X-MICROSOFT-CDO-INTENDEDSTATUS")
            intended[i[1].strip().upper() if i else "(fehlt)"] += 1
            t = prop(c, "TRANSP")
            transp[t[1].strip().upper() if t else "(fehlt)"] += 1
            s = prop(c, "STATUS")
            status[s[1].strip().upper() if s else "(fehlt)"] += 1
            k = prop(c, "CLASS")
            klass[k[1].strip().upper() if k else "(fehlt)"] += 1
            r = prop(c, "RRULE")
            if r:
                m = re.search(r"FREQ=([A-Z]+)", r[1].upper())
                bounded = "UNTIL=" in r[1].upper() or "COUNT=" in r[1].upper()
                freq[f"{m.group(1) if m else '?'} ({'begrenzt' if bounded else 'endlos'})"] += 1
            d = prop(c, "DTSTART")
            if d and date_of(d[1]):
                months.append(date_of(d[1]))
            u = prop(c, "UID")
            if u and u[1].strip():
                by_uid[u[1].strip()].append(c)
            else:
                no_uid += 1

    masters = series = overrides = orphan_uids = orphan_events = 0
    dup_masters = dup_overrides = 0
    with_attendees = with_organizer = with_alarm = with_exdate = with_rdate = 0
    for uid, events in by_uid.items():
        m = [e for e in events if prop(e, "RECURRENCE-ID") is None]
        o = [e for e in events if prop(e, "RECURRENCE-ID") is not None]
        masters += min(len(m), 1)
        dup_masters += max(len(m) - 1, 0)
        series += sum(1 for e in m[:1] if prop(e, "RRULE") or prop(e, "RDATE"))
        overrides += len(o)
        keys = collections.Counter(
            (prop(e, "RECURRENCE-ID")[0].get("TZID", ""), prop(e, "RECURRENCE-ID")[1].strip()) for e in o
        )
        dup_overrides += sum(n - 1 for n in keys.values() if n > 1)
        if not m and o:
            orphan_uids += 1
            orphan_events += len(o)
        for e in events:
            with_attendees += prop(e, "ATTENDEE") is not None
            with_organizer += prop(e, "ORGANIZER") is not None
            with_alarm += any(ch["name"] == "VALARM" for ch in e["children"])
            with_exdate += prop(e, "EXDATE") is not None
            with_rdate += prop(e, "RDATE") is not None

    stats.update({
        "header": header,
        "komponenten": dict(comp_counts),
        "vevents": comp_counts.get("VEVENT", 0),
        "uids": len(by_uid),
        "vevents_ohne_uid": no_uid,
        "einzeltermine": masters - series,
        "serien": series,
        "rrule_freq": dict(freq),
        "overrides": overrides,
        "uids_ohne_master": orphan_uids,
        "overrides_ohne_master": orphan_events,
        "doppelte_master": dup_masters,
        "doppelte_overrides": dup_overrides,
        "mit_exdate": with_exdate,
        "mit_rdate": with_rdate,
        "mit_teilnehmern": with_attendees,
        "mit_organisator": with_organizer,
        "mit_erinnerung": with_alarm,
        "tzids_benutzt": dict(tzid_used),
        "tzids_definiert": dict(tzid_defined),
        "tzids_ohne_vtimezone": sorted(set(tzid_used) - set(tzid_defined)),
        "zeitwerte": dict(value_kinds),
        "busystatus": dict(busy),
        "intendedstatus": dict(intended),
        "transp": dict(transp),
        "status": dict(status),
        "class": dict(klass),
        "x_props_kalender": dict(cal_x),
        "x_props_termine": dict(ev_x),
        "zeitraum": f"{min(months)} bis {max(months)}" if months else None,
    })
    return stats


def stable_hash(body: bytes) -> str:
    """Hash without DTSTAMP lines (many feeds restamp every request)."""
    text = body.decode("utf-8", "replace")
    kept = [l for l in unfold(text) if not l.upper().startswith("DTSTAMP")]
    return hashlib.sha256("\n".join(kept).encode()).hexdigest()


def main() -> None:
    p = argparse.ArgumentParser(description="ICS-Feed-Spike für Lichtung (S3)")
    p.add_argument("--json", action="store_true", help="Ergebnis als JSON ausgeben")
    p.add_argument("--user", help="Benutzername für Basic-Auth (nur falls der Feed eine Anmeldung verlangt)")
    p.add_argument("--wait", type=int, default=5, help="Sekunden zwischen erstem und zweitem Abruf (Standard 5)")
    args = p.parse_args()

    raw = os.environ.get("LICHTUNG_SPIKE_ICS_URL") or getpass.getpass("Feed-URL (wird nicht angezeigt): ")
    url = normalize_url(raw)
    auth = None
    if args.user:
        password = os.environ.get("LICHTUNG_SPIKE_ICS_PASSWORD") or getpass.getpass("Passwort: ")
        auth = "Basic " + base64.b64encode(f"{args.user}:{password}".encode()).decode()

    first = fetch(url, auth, {})
    result: dict = {
        "schema": urllib.parse.urlsplit(url).scheme,
        "url_mit_query": bool(urllib.parse.urlsplit(url).query),
        "abruf_1": {
            "status": first["status"],
            "sekunden": first["seconds"],
            "weiterleitungen": first["redirects"],
            "weiterleitung_auf_anderen_host": first["redirect_host_changed"],
            "content_type": first["headers"].get("content-type"),
            "content_encoding": first["headers"].get("content-encoding"),
            "groesse_bytes": len(first["body"]),
            "ueber_20_mb": len(first["body"]) > MAX_FEED_BYTES,
            "etag": "etag" in first["headers"],
            "etag_schwach": first["headers"].get("etag", "").startswith("W/"),
            "last_modified": "last-modified" in first["headers"],
            "cache_control": first["headers"].get("cache-control"),
        },
    }
    if first["status"] != 200:
        print(json.dumps(result, indent=2, ensure_ascii=False))
        sys.exit(f"Erster Abruf: HTTP {first['status']}, Abbruch.")

    time.sleep(max(args.wait, 0))
    cond = {}
    if "etag" in first["headers"]:
        cond["If-None-Match"] = first["headers"]["etag"]
    if "last-modified" in first["headers"]:
        cond["If-Modified-Since"] = first["headers"]["last-modified"]
    checks = {}
    if cond:
        second = fetch(url, auth, cond)
        checks["beide_bedingungen"] = second["status"]
        if "If-None-Match" in cond:
            checks["nur_if_none_match"] = fetch(url, auth, {"If-None-Match": cond["If-None-Match"]})["status"]
        if "If-Modified-Since" in cond:
            checks["nur_if_modified_since"] = fetch(url, auth, {"If-Modified-Since": cond["If-Modified-Since"]})["status"]
    plain = fetch(url, auth, {})
    result["abruf_2"] = {
        "bedingte_abrufe_status": checks or "keine Validatoren, nicht geprüft",
        "unbedingt_status": plain["status"],
        "inhalt_byte_gleich": plain["body"] == first["body"],
        "inhalt_gleich_ohne_dtstamp": stable_hash(plain["body"]) == stable_hash(first["body"]),
        "etag_gleich": plain["headers"].get("etag") == first["headers"].get("etag"),
    }
    result["inhalt"] = analyze(first["body"])

    if args.json:
        print(json.dumps(result, indent=2, ensure_ascii=False))
        return
    print_report(result)


def print_report(r: dict) -> None:
    a1, a2, s = r["abruf_1"], r["abruf_2"], r["inhalt"]

    def line(k, v):
        if isinstance(v, dict):
            v = ", ".join(f"{kk}: {vv}" for kk, vv in sorted(v.items(), key=lambda x: (-x[1], x[0]) if isinstance(x[1], int) else (0, x[0]))) or "–"
        elif isinstance(v, list):
            v = ", ".join(v) or "–"
        elif isinstance(v, bool):
            v = "ja" if v else "nein"
        print(f"| {k} | {v} |")

    print("| Frage | Ergebnis |")
    print("|---|---|")
    line("Schema / Token in der Query", f"{r['schema']} / {'ja' if r['url_mit_query'] else 'nein'}")
    line("HTTP-Status, Dauer", f"{a1['status']}, {a1['sekunden']} s")
    line("Weiterleitungen (anderer Host?)", f"{a1['weiterleitungen']} ({'ja' if a1['weiterleitung_auf_anderen_host'] else 'nein'})")
    line("Content-Type / -Encoding", f"{a1['content_type'] or '–'} / {a1['content_encoding'] or '–'}")
    line("Größe", f"{a1['groesse_bytes']:,} Bytes".replace(",", "."))
    line("ETag (schwach?) / Last-Modified", f"{'ja' if a1['etag'] else 'nein'} ({'ja' if a1['etag_schwach'] else 'nein'}) / {'ja' if a1['last_modified'] else 'nein'}")
    line("Cache-Control", a1["cache_control"] or "–")
    line("Zweiter Abruf mit Bedingung", a2["bedingte_abrufe_status"])
    line("Inhalt gleich (byte / ohne DTSTAMP)", f"{'ja' if a2['inhalt_byte_gleich'] else 'nein'} / {'ja' if a2['inhalt_gleich_ohne_dtstamp'] else 'nein'}")
    line("ETag stabil", a2["etag_gleich"])
    line("Kopf (PRODID, METHOD, …)", s["header"] and {k: v for k, v in s["header"].items()})
    line("Zeilenenden, BOM, gefaltete Zeilen", f"{s['line_endings']}, {'ja' if s['bom'] else 'nein'}, {s['folded_lines']}")
    line("Komponenten", s["komponenten"])
    line("VEVENTs / UIDs / ohne UID", f"{s['vevents']} / {s['uids']} / {s['vevents_ohne_uid']}")
    line("Einzeltermine / Serien", f"{s['einzeltermine']} / {s['serien']}")
    line("RRULE-Arten", s["rrule_freq"])
    line("Overrides (davon ohne Master)", f"{s['overrides']} ({s['overrides_ohne_master']} in {s['uids_ohne_master']} UIDs)")
    line("Doppelte Master / doppelte Overrides", f"{s['doppelte_master']} / {s['doppelte_overrides']}")
    line("Mit EXDATE / RDATE", f"{s['mit_exdate']} / {s['mit_rdate']}")
    line("Mit Teilnehmern / Organisator / Erinnerung", f"{s['mit_teilnehmern']} / {s['mit_organisator']} / {s['mit_erinnerung']}")
    line("TZIDs benutzt", s["tzids_benutzt"])
    line("TZIDs ohne VTIMEZONE", s["tzids_ohne_vtimezone"])
    line("Zeitwerte", s["zeitwerte"])
    line("BUSYSTATUS", s["busystatus"])
    line("INTENDEDSTATUS", s["intendedstatus"])
    line("TRANSP", s["transp"])
    line("STATUS", s["status"])
    line("CLASS", s["class"])
    line("X-Properties (Kalender)", s["x_props_kalender"])
    line("X-Properties (Termine)", s["x_props_termine"])
    line("Zeitraum (DTSTART, Monat)", s["zeitraum"] or "–")


if __name__ == "__main__":
    main()
