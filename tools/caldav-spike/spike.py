#!/usr/bin/env python3
"""CalDAV scheduling spike for Lichtung (LIC-6 Fastmail, LIC-13 iCloud).

Checks against a real account whether server-side scheduling (RFC 6638)
works: does a PUT with attendees send invitations, does changing the own
PARTSTAT send a reply?

Only the Python standard library is used. Credentials are read from the
environment or prompted for; they are never written anywhere.

    export LICHTUNG_SPIKE_USER=person@example.org
    python3 tools/caldav-spike/spike.py --provider fastmail discover

See README.md next to this file for the full walkthrough.
"""

from __future__ import annotations

import argparse
import base64
import datetime as dt
import getpass
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid
import xml.etree.ElementTree as ET

PROVIDERS = {
    "fastmail": "https://caldav.fastmail.com/dav/",
    "icloud": "https://caldav.icloud.com/",
}

NS = {
    "d": "DAV:",
    "c": "urn:ietf:params:xml:ns:caldav",
    "cs": "http://calendarserver.org/ns/",
    "a": "http://apple.com/ns/ical/",
}
for prefix, uri in NS.items():
    ET.register_namespace(prefix, uri)


MAX_REDIRECTS = 10

AUTH_HINTS = {
    "fastmail": "Fastmail braucht ein App-Passwort (Einstellungen → Privatsphäre & Sicherheit → "
                "App-Passwörter, Zugriff „Kalender (CalDAV)“), nicht das normale Passwort.",
    "icloud": "iCloud braucht ein App-spezifisches Passwort (appleid.apple.com → Anmelden und "
              "Sicherheit → App-spezifische Passwörter), nicht das Passwort der Apple-ID.",
}


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """Hands every redirect back to the caller (as HTTPError).

    urllib follows redirects only for GET/HEAD (PROPFIND/REPORT get a 301
    back) and turns the method into GET; Client.request follows them itself.
    """

    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: D401
        return None


_OPENER = urllib.request.build_opener(_NoRedirect)


def credentials_allowed(url: str, base: str) -> bool:
    """Sends credentials to the base URL's origin and, over HTTPS only, to
    iCloud hosts (``caldav.icloud.com`` redirects to ``pNN-caldav.icloud.com``)."""
    u, b = urllib.parse.urlsplit(url), urllib.parse.urlsplit(base)

    def origin(x):
        return (x.scheme, (x.hostname or "").lower(), x.port or (443 if x.scheme == "https" else 80))

    if origin(u) == origin(b):
        return True
    host = (u.hostname or "").lower()
    return u.scheme == "https" and (host == "icloud.com" or host.endswith(".icloud.com"))


class Client:
    def __init__(self, base: str, user: str, password: str, verbose: bool, provider: str = "") -> None:
        self.base = base
        self.verbose = verbose
        self.provider = provider
        token = base64.b64encode(f"{user}:{password}".encode()).decode()
        self.auth = f"Basic {token}"
        # URL that answered the last request (after redirects); relative
        # hrefs in its body are resolved against it.
        self.last_url = base

    def request(self, method: str, url: str, body: str | None = None,
                headers: dict[str, str] | None = None) -> tuple[int, dict[str, str], str]:
        url = urllib.parse.urljoin(self.base, url)
        data = body.encode() if body is not None else None
        for _ in range(MAX_REDIRECTS + 1):
            req = urllib.request.Request(url, data=data, method=method)
            if credentials_allowed(url, self.base):
                req.add_header("Authorization", self.auth)
            req.add_header("User-Agent", "lichtung-spike/0.1")
            for k, v in (headers or {}).items():
                req.add_header(k, v)
            try:
                with _OPENER.open(req, timeout=30) as resp:
                    status, hdrs, text = resp.status, dict(resp.headers), resp.read().decode("utf-8", "replace")
            except urllib.error.HTTPError as e:
                status, hdrs, text = e.code, dict(e.headers), e.read().decode("utf-8", "replace")
            if self.verbose:
                print(f"--- {method} {url} -> {status}", file=sys.stderr)
                print(text[:4000], file=sys.stderr)
            location = hdrs.get("Location") or hdrs.get("location")
            if status in (301, 302, 303, 307, 308) and location:
                url = urllib.parse.urljoin(url, location)
                if not url.startswith(("https://", "http://")):
                    sys.exit(f"Weiterleitung auf {url} nicht unterstützt")
                if status == 303:
                    method, data = "GET", None
                continue
            break
        else:
            sys.exit("Zu viele Weiterleitungen")
        self.last_url = url
        if status == 401:
            hint = AUTH_HINTS.get(self.provider, "Benutzername oder App-Passwort prüfen.")
            sys.exit(f"Anmeldung abgelehnt (401). {hint}")
        return status, hdrs, text

    def absolute(self, href: str) -> str:
        """An href from the last response as an absolute URL."""
        return urllib.parse.urljoin(self.last_url, href.strip())

    def propfind(self, url: str, props: str, depth: str = "0") -> ET.Element:
        body = (f'<?xml version="1.0" encoding="utf-8"?>'
                f'<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" '
                f'xmlns:cs="http://calendarserver.org/ns/" xmlns:a="http://apple.com/ns/ical/">'
                f'<d:prop>{props}</d:prop></d:propfind>')
        status, _, text = self.request("PROPFIND", url, body,
                                       {"Depth": depth, "Content-Type": "application/xml; charset=utf-8"})
        if status != 207:
            sys.exit(f"PROPFIND {url} lieferte {status} statt 207")
        return ET.fromstring(text)


def href_of(el: ET.Element, path: str) -> str | None:
    found = el.find(path, NS)
    if found is None:
        return None
    h = found.find("d:href", NS)
    return h.text.strip() if h is not None and h.text else None


def discover(client: Client) -> dict:
    # Follows /.well-known/caldav (iCloud: 301) like Lichtung, else the base.
    status, _, _ = client.request("PROPFIND", "/.well-known/caldav",
                                  '<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:">'
                                  '<d:prop><d:current-user-principal/></d:prop></d:propfind>',
                                  {"Depth": "0", "Content-Type": "application/xml; charset=utf-8"})
    start = client.last_url if status == 207 else ""
    root = client.propfind(start, "<d:current-user-principal/>")
    principal = href_of(root, ".//d:current-user-principal")
    if not principal:
        sys.exit("current-user-principal nicht gefunden")
    principal = client.absolute(principal)
    root = client.propfind(principal, "<c:calendar-home-set/><c:schedule-inbox-URL/>"
                                      "<c:schedule-outbox-URL/><c:calendar-user-address-set/>"
                                      "<d:displayname/>")

    def absolute(href: str | None) -> str | None:
        return client.absolute(href) if href else None

    # iCloud: the calendar home lives on a partition host,
    # e.g. https://p42-caldav.icloud.com:443/<dsid>/calendars/.
    home = absolute(href_of(root, ".//c:calendar-home-set"))
    preferred = [h.text.strip() for h in root.findall(".//c:calendar-user-address-set/d:href", NS)
                 if h.text and h.get("preferred") == "1"]
    addresses = preferred + [h.text.strip() for h in root.findall(".//c:calendar-user-address-set/d:href", NS)
                             if h.text and h.text.strip() not in preferred]
    info = {
        "principal": principal,
        "home": home,
        "inbox": absolute(href_of(root, ".//c:schedule-inbox-URL")),
        "outbox": absolute(href_of(root, ".//c:schedule-outbox-URL")),
        "addresses": addresses,
    }
    if not home:
        sys.exit("calendar-home-set nicht gefunden")
    status, headers, _ = client.request("OPTIONS", home)
    info["dav_header"] = headers.get("DAV") or headers.get("Dav") or ""
    root = client.propfind(home, "<d:displayname/><d:resourcetype/><c:supported-calendar-component-set/>"
                                 "<d:current-user-privilege-set/><d:sync-token/><cs:getctag/>"
                                 "<c:schedule-calendar-transp/><a:calendar-color/><cs:invite/>", depth="1")
    calendars = []
    for resp in root.findall("d:response", NS):
        if resp.find(".//d:resourcetype/c:calendar", NS) is None:
            continue
        name = resp.find(".//d:displayname", NS)
        comps = [c.get("name") for c in resp.findall(".//c:supported-calendar-component-set/c:comp", NS)]
        privs = [p[0].tag.split("}")[1] for p in resp.findall(".//d:current-user-privilege-set/d:privilege", NS) if len(p)]
        if resp.find(".//d:resourcetype/cs:shared", NS) is not None:
            shared = "geteilt mit mir"
        elif resp.find(".//d:resourcetype/cs:shared-owner", NS) is not None:
            shared = "von mir geteilt"
        elif resp.find(".//cs:invite/cs:organizer", NS) is not None:
            shared = "geteilt (invite)"
        else:
            shared = ""
        calendars.append({
            "href": client.absolute(resp.find("d:href", NS).text),
            "name": name.text if name is not None else "",
            "components": comps,
            "writable": any(p in ("write", "write-content", "all") for p in privs),
            "shared": shared,
            "sync_token": resp.find(".//d:sync-token", NS) is not None,
        })
    info["calendars"] = calendars
    return info


def own_address(info: dict) -> str:
    # The preferred address (iCloud marks it with preferred="1") comes first.
    for a in info["addresses"]:
        if a.lower().startswith("mailto:"):
            return a
    sys.exit("Keine mailto:-Adresse in calendar-user-address-set")


def ics_event(uid: str, organizer: str, attendee: str, start: dt.datetime) -> str:
    end = start + dt.timedelta(minutes=30)
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    fmt = "%Y%m%dT%H%M%S"
    lines = [
        "BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Lichtung//Spike//DE",
        "BEGIN:VEVENT",
        f"UID:{uid}", f"DTSTAMP:{stamp}",
        f"DTSTART;TZID=Europe/Berlin:{start.strftime(fmt)}",
        f"DTEND;TZID=Europe/Berlin:{end.strftime(fmt)}",
        "SUMMARY:Lichtung-Spike: Testeinladung",
        "DESCRIPTION:Automatischer Test von Lichtung (LIC-6, LIC-13). Kann gelöscht werden.",
        f"ORGANIZER:{organizer}",
        f"ATTENDEE;ROLE=CHAIR;PARTSTAT=ACCEPTED:{organizer}",
        f"ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{attendee}",
        "END:VEVENT", "END:VCALENDAR", "",
    ]
    return "\r\n".join(lines)


def unfold(text: str) -> list[str]:
    out: list[str] = []
    for line in text.replace("\r\n", "\n").split("\n"):
        if line.startswith((" ", "\t")) and out:
            out[-1] += line[1:]
        else:
            out.append(line)
    return out


def split_prop(line: str) -> tuple[str, str]:
    """Splits a content line into name+params and value, honouring quotes."""
    quoted = False
    for i, ch in enumerate(line):
        if ch == '"':
            quoted = not quoted
        elif ch == ":" and not quoted:
            return line[:i], line[i + 1:]
    return line, ""


def cmd_discover(client: Client, args) -> None:
    info = discover(client)
    print(f"Principal:        {info['principal']}")
    print(f"Calendar-Home:    {info['home']}")
    print(f"Schedule-Inbox:   {info['inbox']}")
    print(f"Schedule-Outbox:  {info['outbox']}")
    print(f"Adressen:         {', '.join(info['addresses'])}")
    auto = "calendar-auto-schedule" in info["dav_header"]
    print(f"DAV-Header:       {info['dav_header']}")
    print(f"=> calendar-auto-schedule angekündigt: {'JA' if auto else 'NEIN'}")
    print("\nKalender:")
    for c in info["calendars"]:
        flags = []
        flags.append("schreibbar" if c["writable"] else "nur lesen")
        if c["shared"]:
            flags.append(c["shared"])
        if c["sync_token"]:
            flags.append("sync-token")
        print(f"  {c['name']!r:30} {','.join(c['components']):15} {' '.join(flags):35} {c['href']}")


def cmd_invite(client: Client, args) -> None:
    info = discover(client)
    organizer = f"mailto:{args.organizer}" if args.organizer else own_address(info)
    cal = args.calendar or next((c["href"] for c in info["calendars"]
                                 if c["writable"] and "VEVENT" in c["components"] and not c["shared"]), None)
    if not cal:
        sys.exit("Kein beschreibbarer Kalender gefunden; --calendar angeben")
    start = (dt.datetime.now() + dt.timedelta(days=2)).replace(hour=10, minute=0, second=0, microsecond=0)
    uid = f"{uuid.uuid4()}@lichtung-spike"
    href = urllib.parse.urljoin(cal if cal.endswith("/") else cal + "/", f"{uid}.ics")
    status, headers, text = client.request("PUT", href, ics_event(uid, organizer, args.attendee, start),
                                           {"Content-Type": "text/calendar; charset=utf-8",
                                            "If-None-Match": "*"})
    print(f"PUT {href} -> {status}")
    print(f"  ETag: {headers.get('ETag')}  Schedule-Tag: {headers.get('Schedule-Tag')}")
    if status not in (200, 201, 204):
        print(text)
        sys.exit("Anlegen fehlgeschlagen")
    status, headers, text = client.request("GET", href)
    print("\nGespeicherte Fassung (ATTENDEE-Zeilen):")
    for line in unfold(text):
        if line.startswith(("ATTENDEE", "ORGANIZER")):
            print(f"  {line}")
    print("\nSCHEDULE-STATUS=1.2 am Teilnehmer heißt: Server hat die Einladung zugestellt.")
    print(f"Jetzt im Postfach von {args.attendee} nachsehen, ob die Einladung angekommen ist.")
    print(f"Aufräumen später: spike.py --provider {args.provider} delete --href {href}")


def cmd_requests(client: Client, args) -> None:
    info = discover(client)
    me = {a.lower() for a in info["addresses"]}
    now = dt.datetime.now(dt.timezone.utc)
    rng = (now.strftime("%Y%m%dT%H%M%SZ"), (now + dt.timedelta(days=args.days)).strftime("%Y%m%dT%H%M%SZ"))
    body = ('<?xml version="1.0" encoding="utf-8"?>'
            '<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">'
            '<d:prop><d:getetag/><c:calendar-data/></d:prop>'
            '<c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">'
            f'<c:time-range start="{rng[0]}" end="{rng[1]}"/>'
            '</c:comp-filter></c:comp-filter></c:filter></c:calendar-query>')
    found = 0
    for cal in info["calendars"]:
        if "VEVENT" not in cal["components"]:
            continue
        status, _, text = client.request("REPORT", cal["href"], body,
                                         {"Depth": "1", "Content-Type": "application/xml; charset=utf-8"})
        if status != 207:
            continue
        for resp in ET.fromstring(text).findall("d:response", NS):
            data = resp.find(".//c:calendar-data", NS)
            if data is None or not data.text:
                continue
            lines = unfold(data.text)
            for line in lines:
                if not line.startswith("ATTENDEE"):
                    continue
                params, value = split_prop(line)
                if value.lower() in me and "PARTSTAT=NEEDS-ACTION" in params.upper():
                    summary = next((split_prop(l)[1] for l in lines if l.startswith("SUMMARY")), "")
                    start = next((l for l in lines if l.startswith("DTSTART")), "")
                    print(f"{summary!r}  {start}\n  {client.absolute(resp.find('d:href', NS).text)}")
                    found += 1
                    break
    print(f"\n{found} offene Anfrage(n) in den nächsten {args.days} Tagen.")


def cmd_respond(client: Client, args) -> None:
    info = discover(client)
    me = {a.lower() for a in info["addresses"]}
    if args.me:
        me.add(f"mailto:{args.me}".lower())
    status, headers, text = client.request("GET", args.href)
    if status != 200:
        sys.exit(f"GET lieferte {status}")
    etag = headers.get("ETag")
    out, changed = [], 0
    for line in unfold(text):
        if line.startswith("ATTENDEE") and split_prop(line)[1].lower() in me:
            params, value = split_prop(line)
            parts = [p for p in params.split(";") if not p.upper().startswith(("PARTSTAT=", "RSVP=", "SCHEDULE-STATUS="))]
            line = ";".join(parts + [f"PARTSTAT={args.status}"]) + ":" + value
            changed += 1
        out.append(line)
    if not changed:
        sys.exit("Eigene Adresse ist nicht unter den Teilnehmern")
    body = "\r\n".join(l for l in out if l) + "\r\n"
    hdrs = {"Content-Type": "text/calendar; charset=utf-8"}
    if etag:
        hdrs["If-Match"] = etag
    status, headers, resp_text = client.request("PUT", args.href, body, hdrs)
    print(f"PUT {args.href} (If-Match {etag}) -> {status}")
    print(f"  ETag: {headers.get('ETag')}  Schedule-Tag: {headers.get('Schedule-Tag')}")
    if status not in (200, 201, 204):
        print(resp_text)
        sys.exit("Antwort fehlgeschlagen")
    print("Jetzt beim Organisator nachsehen, ob die Antwort angekommen ist.")


def cmd_delete(client: Client, args) -> None:
    status, headers, _ = client.request("GET", args.href)
    etag = headers.get("ETag") if status == 200 else None
    hdrs = {"If-Match": etag} if etag else {}
    status, _, text = client.request("DELETE", args.href, None, hdrs)
    print(f"DELETE {args.href} -> {status}")
    if status not in (200, 204):
        print(text)


def main() -> None:
    p = argparse.ArgumentParser(description="CalDAV-Scheduling-Spike für Lichtung")
    p.add_argument("--provider", choices=sorted(PROVIDERS), default="fastmail")
    p.add_argument("--base-url", help="CalDAV-Basis-URL statt der des Anbieters")
    p.add_argument("-v", "--verbose", action="store_true", help="Rohantworten auf stderr ausgeben")
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("discover", help="Principal, Postfächer, DAV-Fähigkeiten und Kalender anzeigen")
    s = sub.add_parser("invite", help="Termin mit Teilnehmer anlegen (prüft: wird eingeladen?)")
    s.add_argument("--attendee", required=True, help="E-Mail des Teilnehmers (eigene Zweitadresse)")
    s.add_argument("--calendar", help="href des Zielkalenders (Standard: erster eigener beschreibbarer)")
    s.add_argument("--organizer", help="Organisator-Adresse erzwingen (nur für Tests ohne Scheduling)")
    s = sub.add_parser("requests", help="offene Terminanfragen (PARTSTAT=NEEDS-ACTION) auflisten")
    s.add_argument("--days", type=int, default=60)
    s = sub.add_parser("respond", help="eigene PARTSTAT setzen (prüft: geht Antwort raus?)")
    s.add_argument("--href", required=True)
    s.add_argument("--me", help="eigene Adresse zusätzlich annehmen (nur für Tests)")
    s.add_argument("--status", choices=["ACCEPTED", "TENTATIVE", "DECLINED"], default="ACCEPTED")
    s = sub.add_parser("delete", help="Testtermin löschen")
    s.add_argument("--href", required=True)
    args = p.parse_args()

    user = os.environ.get("LICHTUNG_SPIKE_USER") or input("Benutzername: ")
    password = os.environ.get("LICHTUNG_SPIKE_PASSWORD") or getpass.getpass("App-Passwort: ")
    client = Client(args.base_url or PROVIDERS[args.provider], user, password, args.verbose, args.provider)
    {"discover": cmd_discover, "invite": cmd_invite, "requests": cmd_requests,
     "respond": cmd_respond, "delete": cmd_delete}[args.cmd](client, args)


if __name__ == "__main__":
    main()
