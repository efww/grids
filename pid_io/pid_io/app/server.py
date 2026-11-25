"""Local web UI for the IO tool.

    python -m pid_io.app            (opens http://127.0.0.1:8765)

Everything stays on this PC: the server listens on 127.0.0.1 only and keeps
each project in its own folder (PID_IO_HOME or ./projects).
"""
from __future__ import annotations

import json
import os
import re
import shutil
import threading
import time
import traceback
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlparse

import pymupdf as fitz

from ..ai import providers as ai
from ..ai import tasks as ait
from ..concept import Concept
from ..export import compare, load_reference, write_template
from ..extract import load_protocol
from ..pipeline import ORIGINS, PROTO, apply_overrides, compute, impact, load_extract, run_extract, summarize

STATIC = Path(__file__).resolve().parent / "static"
HOME = Path(os.environ.get("PID_IO_HOME", Path.cwd() / "projects")).resolve()
KINDS = {"pdf": ".pdf", "template": ".xlsx", "ref": ".xlsx", "concept": ".xlsx"}
COLORS = {"FIELD": (0.08, 0.68, 0.38), "DCS_SHARED": (0.31, 0.63, 0.88), "PANEL_CR": (0.31, 0.63, 0.88),
          "PANEL_BEHIND": (0.5, 0.5, 0.5), "PLC": (0.49, 0.4, 0.8), "ESD_INTERLOCK": (0.82, 0.2, 0.36),
          "DCS_INTERLOCK": (1, 0.73, 0), "HW_INTERLOCK": (1, 0.73, 0)}

JOBS: dict[str, dict] = {}
CACHE: dict[str, dict] = {}
LOCK = threading.Lock()


# ------------------------------------------------------------------ projects
def safe_name(name: str) -> str:
    name = re.sub(r"[\\\\/:*?\"<>|]+", "_", name).strip(" .")
    if not name:
        raise ValueError("프로젝트 이름이 비어 있습니다")
    return name[:80]


def pdir(name: str) -> Path:
    d = (HOME / safe_name(name)).resolve()
    if HOME not in d.parents:
        raise ValueError("bad project")
    return d


def meta(name: str) -> dict:
    f = pdir(name) / "project.json"
    return json.loads(f.read_text(encoding="utf-8")) if f.exists() else {}


def save_meta(name: str, m: dict):
    (pdir(name) / "project.json").write_text(json.dumps(m, ensure_ascii=False, indent=1), encoding="utf-8")


def create_project(name: str):
    d = pdir(name)
    if d.exists():
        raise ValueError("같은 이름의 프로젝트가 있습니다")
    for sub in ("input", "cache", "output"):
        (d / sub).mkdir(parents=True)
    shutil.copy(PROTO / "concept_default.xlsx", d / "concept.xlsx")
    save_meta(name, {"name": safe_name(name), "created": time.strftime("%Y-%m-%d %H:%M"),
                     "files": {}, "scenarios": []})


def concept(name: str) -> Concept:
    c = CACHE.setdefault(name, {})
    if "concept" not in c:
        c["concept"] = Concept(pdir(name) / "concept.xlsx")
    return c["concept"]


def extracted(name: str):
    c = CACHE.setdefault(name, {})
    f = pdir(name) / "extract.json"
    if "extract" not in c and f.exists():
        c["extract"] = load_extract(f)
    return c.get("extract")


def computed(name: str):
    ex = extracted(name)
    if not ex:
        return None
    data, syms, notes = ex
    con = concept(name)
    rows, log = compute(syms, notes, con, overrides=overrides(name))
    return data, syms, notes, con, rows, log


def overrides(name: str):
    return ait.load(pdir(name) / "overrides.json", {"symbols": {}, "rules": []})


# ------------------------------------------------------------------ AI
def ai_view(name: str):
    ex = extracted(name)
    st = ai.load_settings(HOME)
    out = {"provider": st["provider"], "suggestions": ait.load(pdir(name) / "ai_suggestions.json", []),
           "pending": ait.load(pdir(name) / "ai_pending.json", None), "job": JOBS.get(name + ":ai", {"state": "idle"})}
    if ex:
        rules = apply_overrides(ex[1], load_protocol(PROTO / "io_rules.yaml"), overrides(name))
        syms, rl = rules
        unknown = ait.unknown_funcs(syms, rl["rules"])
        out["counts"] = {"symbols": len(ait.symbol_items(syms, set(unknown))), "funcs": len(unknown)}
    return out


def ai_run(name: str, kind: str):
    ex = extracted(name)
    if not ex:
        raise ValueError("먼저 추출을 실행해 주세요")
    d = pdir(name)
    syms, rl = apply_overrides(ex[1], load_protocol(PROTO / "io_rules.yaml"), overrides(name))
    unknown = ait.unknown_funcs(syms, rl["rules"])
    if kind == "symbols":
        prompt, files, asked = ait.build_symbols(d / "input" / meta(name)["files"]["pdf"], syms, set(unknown),
                                                 d / "cache" / "ai_crops")
    elif kind == "funcs":
        prompt, asked = ait.build_funcs(unknown)
        files = []
    else:
        raise ValueError("kind")
    if not asked:
        raise ValueError("AI에게 물어볼 항목이 없습니다")
    st = ai.load_settings(HOME)
    task = {"task_id": time.strftime("%H%M%S"), "kind": kind, "asked": asked, "prompt": prompt,
            "crops": [f.name for f in files]}
    if st["provider"] == "manual":
        ait.save(d / "ai_pending.json", task)
        return {"manual": True, **task}
    key = name + ":ai"
    JOBS[key] = {"state": "running", "msg": f"Copilot에 {len(asked)}건 질문 중", "started": time.time()}

    def work():
        try:
            ans = ai.ask(st, prompt)
            ai_accept_answer(name, task, ans, source=st["provider"])
            JOBS[key].update(state="done", msg="완료", elapsed=round(time.time() - JOBS[key]["started"]))
        except Exception as e:
            JOBS[key].update(state="error", msg=str(e))

    threading.Thread(target=work, daemon=True).start()
    return {"manual": False, "started": True}


def ai_accept_answer(name: str, task: dict, answer: str, source="manual"):
    d = pdir(name)
    if task["kind"] == "symbols":
        items = ait.parse_symbols(answer, set(task["asked"]))
        ex = extracted(name)
        by = {s.sid: s for s in ex[1]} if ex else {}
        for it in items:
            s = by.get(it["id"])
            if s:
                it.update(page=s.page, dwg=s.dwg, coord=f"{round(s.cx)},{round(s.cy)}", text=f"{s.func} {s.number}")
    else:
        items = ait.parse_funcs(answer, set(task["asked"]))
    if not items:
        raise ValueError("답변에서 유효한 항목을 찾지 못했습니다 (형식 확인)")
    ait.add_suggestions(d, task["kind"], items, source)
    (d / "ai_pending.json").unlink(missing_ok=True)
    return len(items)


# ------------------------------------------------------------------ extraction job
def start_extract(name: str):
    m = meta(name)
    pdf = m.get("files", {}).get("pdf")
    if not pdf:
        raise ValueError("P&ID PDF를 먼저 올려 주세요")
    with LOCK:
        if JOBS.get(name, {}).get("state") == "running":
            return
        JOBS[name] = {"state": "running", "page": 0, "total": 0, "symbols": 0, "started": time.time(), "msg": "레전드 학습"}

    def work():
        try:
            def prog(p, n, k):
                j = JOBS[name]
                j.update(page=p, total=n, symbols=j["symbols"] + k, msg=f"심볼 인식 {p}/{n} 페이지")
            d = pdir(name)
            run_extract(d / "input" / pdf, d / "extract.json", progress=prog)
            CACHE.pop(name, None)
            shutil.rmtree(d / "cache", ignore_errors=True)
            (d / "cache").mkdir(exist_ok=True)
            m2 = meta(name)
            m2["extracted"] = time.strftime("%Y-%m-%d %H:%M")
            save_meta(name, m2)
            JOBS[name].update(state="done", msg="완료", elapsed=round(time.time() - JOBS[name]["started"]))
        except Exception as e:  # report to the UI
            JOBS[name].update(state="error", msg=str(e), trace=traceback.format_exc())

    threading.Thread(target=work, daemon=True).start()


# ------------------------------------------------------------------ views
def project_state(name: str):
    m = meta(name)
    d = pdir(name)
    st = {"meta": m, "job": JOBS.get(name, {"state": "idle"}), "extracted": (d / "extract.json").exists()}
    ex = extracted(name)
    if ex:
        data, syms, notes = ex
        by = {}
        for s in syms:
            by[s.cls] = by.get(s.cls, 0) + 1
        st["extract"] = {
            "pdf": data["pdf"], "pages": data["page_count"], "symbols": len(syms),
            "by_class": by,
            "esd_linked": sum(1 for s in syms if s.role == "device" and any("IS" in l.split(":")[1] for l in s.links)),
            "pumps": sum(1 for s in syms if s.func == "EQUIP:P"),
            "packages": sum(1 for s in syms if s.func == "EQUIP:PK"),
            "vendor_symbols": sum(1 for s in syms if s.vendor),
            "vendor_areas": sorted({s.vendor for s in syms if s.vendor}),
            "notes": sum(len(n.notes) for n in notes.values()),
            "train_sheets": [d for d, n in notes.items() if n.copies],
        }
    return st


def concept_view(name: str):
    con = concept(name)
    groups = []
    for iid, it in con.items.items():
        if not groups or groups[-1]["title"] != it["group"]:
            groups.append({"title": it["group"], "items": []})
        groups[-1]["items"].append({"id": iid, **{k: it[k] for k in ("name", "type", "value", "default", "choices", "effect", "basis")}})
    pk = []
    ex = extracted(name)
    seen = set()
    if ex:
        for s in ex[1]:
            if s.func == "EQUIP:PK":
                typ, conn, prow, src = con.package_typical(s.number)
                units = s.notes[0].split(":", 1)[1] if s.notes else ""
                pk.append({"tag": s.number, "dwg": s.dwg, "title": s.title, "units": units,
                           "conn": conn, "typical": typ, "source": src,
                           "qty": (prow or {}).get("qty") or max(1, len([u for u in units.split("/") if u]))})
                seen.add(s.number)
    for p in con.packages:
        if not any(p["tag"].startswith(t) for t in seen):
            pk.append({"tag": p["tag"], "dwg": p.get("dwg"), "title": p.get("name"), "units": "",
                       "conn": p.get("conn"), "typical": p.get("typical"), "source": "Package_Concept",
                       "qty": p.get("qty")})
    return {"groups": groups, "packages": pk, "changed": len(con.changed())}


def summary_view(name: str):
    c = computed(name)
    if not c:
        return {"ready": False}
    data, syms, notes, con, rows, log = c
    s = summarize(rows, con)
    s["ready"] = True
    s["origins"] = ORIGINS
    s["scenarios"] = meta(name).get("scenarios", [])
    return s


REASONS = [("ESD interlock", "ESD 연결"), ("interlock link via actuator", "ESD 연결 추정"),
           ("벤더 경계", "벤더 경계 안"), ("same tag", "중복 태그"), ("ONLY ONE IS SHOWN", "대수 확인 필요"),
           ("function code not in", "미분류 기능코드"), ("IO 제외", "벤더 PLC 내부"), ("물량 미상", "패키지 물량 미상")]


def review_view(name: str, q: dict):
    c = computed(name)
    if not c:
        return {"ready": False}
    data, syms, notes, con, rows, log = c
    items, counts = [], {}
    for r in rows:
        if r.kind == "function" or r.origin in ("스페어", "트레인복제"):
            continue
        text = "; ".join(r.inform)
        tags = [label for key, label in REASONS if key in text]
        if r.status == "확정" and not tags:
            continue
        if r.status != "확정" and not tags:
            tags = ["파생/가정 행"] if r.derived else ["도형 일치도 낮음"]
        for t in tags:
            counts[t] = counts.get(t, 0) + 1
        items.append({"tag": r.tag, "desc": r.desc, "system": r.system, "io": r.io, "status": r.status,
                      "origin": r.origin, "dwg": r.dwg, "page": r.page, "coord": r.coord, "vendor": r.vendor_pkg,
                      "inform": text, "basis": r.basis, "reasons": tags})
    ov = overrides(name)["symbols"]
    for s in syms:
        if s.role == "candidate" and s.sid not in ov:
            counts["미인식 후보"] = counts.get("미인식 후보", 0) + 1
            items.append({"tag": f"{s.func}-{s.number}", "desc": "규칙에 맞지 않은 태그 모양 글자", "system": "?", "io": "-",
                          "status": "불명", "origin": "도면인식", "dwg": s.dwg, "page": s.page,
                          "coord": f"{round(s.cx)},{round(s.cy)}", "vendor": s.vendor, "inform": "; ".join(s.notes),
                          "basis": "", "reasons": ["미인식 후보"]})
    f = q.get("reason", [""])[0]
    if f:
        items = [i for i in items if f in i["reasons"]]
    pages = sorted({(s.page, s.dwg, s.title) for s in syms if s.page})
    notes_list = [{"dwg": d, "text": t, "effect": e} for d, n in notes.items() for t, e in n.notes]
    return {"ready": True, "counts": counts, "items": items[:600], "total": len(items),
            "pages": [{"page": p, "dwg": d, "title": t} for p, d, t in pages],
            "page_size": data["page_size"], "notes": notes_list}


def page_png(name: str, page: int) -> Path:
    d = pdir(name)
    out = d / "cache" / f"page_{page}.png"
    if out.exists():
        return out
    m = meta(name)
    ex = extracted(name)
    doc = fitz.open(d / "input" / m["files"]["pdf"])
    pg = doc[page - 1]
    inv = ~pg.rotation_matrix
    from ..boundary import find_regions, legend_width
    for rg in find_regions(pg, legend_width(doc)):
        sh = pg.new_shape()
        sh.draw_rect(rg.rect * inv)
        sh.finish(color=(0.49, 0.4, 0.8), width=3, dashes="[12 4 3 4] 0")
        sh.commit()
    for s in (ex[1] if ex else []):
        if s.page != page or s.role == "equipment":
            continue
        r = s.radius * 1.15
        sh = pg.new_shape()
        sh.draw_rect(fitz.Rect(s.cx - r, s.cy - r, s.cx + r, s.cy + r) * inv)
        sh.finish(color=COLORS.get(s.cls, (0.3, 0.3, 0.3)), width=1.8)
        sh.commit()
    pg.get_pixmap(dpi=60).save(out)
    return out


def export(name: str, body: dict):
    c = computed(name)
    if not c:
        raise ValueError("먼저 추출을 실행해 주세요")
    data, syms, notes, con, rows, log = c
    d = pdir(name)
    m = meta(name)
    files = m.get("files", {})
    if not files.get("template"):
        raise ValueError("인덱스 양식(xlsx)을 올려 주세요")
    stamp = time.strftime("%Y%m%d_%H%M")
    out = d / "output" / f"Instrument_Index_{safe_name(name)}_{stamp}.xlsx"
    rev = re.search(r"Rev\.?\s*([A-Z0-9]+)", files["pdf"], re.I)
    write_template(rows, d / "input" / files["template"], out, files["pdf"], rev.group(1) if rev else "", con, log)
    made = [out.name]
    if files.get("ref"):
        from ..extract import page_texts
        compare(rows, load_reference(d / "input" / files["ref"]), out, page_texts(d / "input" / files["pdf"]))
    if body.get("overlay"):
        from ..overlay import write_overlay
        ov = d / "output" / f"P&ID_검토용_{stamp}.pdf"
        write_overlay(d / "input" / files["pdf"], syms, ov)
        made.append(ov.name)
    return {"files": made}


# ------------------------------------------------------------------ http
class H(BaseHTTPRequestHandler):
    server_version = "pid_io"

    def log_message(self, *a):
        pass

    def _send(self, code, body, ctype="application/json; charset=utf-8", extra=None):
        if isinstance(body, (dict, list)):
            body = json.dumps(body, ensure_ascii=False).encode("utf-8")
        elif isinstance(body, str):
            body = body.encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def _json(self):
        n = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(n) or b"{}")

    def _route(self, method):
        u = urlparse(self.path)
        parts = [unquote(p) for p in u.path.strip("/").split("/") if p]
        q = parse_qs(u.query)
        try:
            if method == "GET" and (not parts or parts[0] in ("index.html",)):
                return self._send(200, (STATIC / "index.html").read_bytes(), "text/html; charset=utf-8")
            if parts[:1] != ["api"]:
                return self._send(404, {"error": "not found"})
            if parts[1:] == ["projects"]:
                if method == "GET":
                    HOME.mkdir(parents=True, exist_ok=True)
                    out = []
                    for d in sorted(HOME.iterdir()):
                        if (d / "project.json").exists():
                            mm = json.loads((d / "project.json").read_text(encoding="utf-8"))
                            out.append({"name": d.name, "created": mm.get("created"), "extracted": mm.get("extracted"),
                                        "files": mm.get("files", {})})
                    return self._send(200, {"projects": out, "home": str(HOME)})
                create_project(self._json()["name"])
                return self._send(200, {"ok": True})
            if parts[1] == "ai":                          # global AI settings
                if parts[2:] == ["settings"]:
                    if method == "POST":
                        st = ai.load_settings(HOME)
                        st.update(self._json())
                        ai.save_settings(HOME, st)
                    return self._send(200, ai.load_settings(HOME))
                if parts[2:] == ["test"]:
                    try:
                        ans = ai.test(ai.load_settings(HOME))
                        ok = ait.extract_json(ans).get("ok") is True
                        return self._send(200, {"ok": ok, "answer": ans[:500]})
                    except ai.NeedsManual:
                        return self._send(200, {"ok": True, "answer": "수동 모드: 테스트할 연결이 없습니다"})
            name, act = parts[2], parts[3] if len(parts) > 3 else ""
            if act == "ai":
                sub = parts[4] if len(parts) > 4 else ""
                if sub == "" and method == "GET":
                    return self._send(200, ai_view(name))
                if sub == "run" and method == "POST":
                    return self._send(200, ai_run(name, self._json()["kind"]))
                if sub == "answer" and method == "POST":
                    b = self._json()
                    task = ait.load(pdir(name) / "ai_pending.json", None)
                    if not task or task["task_id"] != b.get("task_id"):
                        raise ValueError("대기 중인 질문이 없습니다. 다시 실행해 주세요")
                    n = ai_accept_answer(name, task, b.get("text", ""), "manual")
                    return self._send(200, {"ok": True, "added": n})
                if sub == "decide" and method == "POST":
                    b = self._json()
                    d = pdir(name)
                    if b.get("min_conf") is not None:
                        for x in ait.load(d / "ai_suggestions.json", []):
                            if x["status"] == "pending" and x.get("confidence", 0) >= float(b["min_conf"]):
                                ait.decide(d, x["sid"], True)
                    else:
                        ait.decide(d, b["sid"], bool(b["accept"]))
                    return self._send(200, ai_view(name))
                if sub == "crop":
                    f = pdir(name) / "cache" / "ai_crops" / Path(parts[5]).name
                    return self._send(200, f.read_bytes(), "image/png")
            if act == "" and method == "GET":
                return self._send(200, project_state(name))
            if act == "upload" and method == "POST":
                kind = q.get("kind", [""])[0]
                if kind not in KINDS:
                    raise ValueError("kind")
                fn = Path(unquote(self.headers.get("X-Filename", f"{kind}{KINDS[kind]}"))).name
                if not fn.lower().endswith(KINDS[kind]):
                    raise ValueError(f"{KINDS[kind]} 파일이 필요합니다")
                n = int(self.headers.get("Content-Length") or 0)
                data = self.rfile.read(n)
                d = pdir(name)
                if kind == "concept":
                    (d / "concept.xlsx").write_bytes(data)
                    CACHE.get(name, {}).pop("concept", None)
                    Concept(d / "concept.xlsx")        # validate
                else:
                    (d / "input" / fn).write_bytes(data)
                m = meta(name)
                m.setdefault("files", {})[kind] = fn if kind != "concept" else "concept.xlsx"
                if kind == "pdf":
                    doc = fitz.open(d / "input" / fn)
                    m["pdf_info"] = {"pages": doc.page_count,
                                     "vector": sum(1 for p in doc if p.get_text().strip()) > doc.page_count / 2}
                save_meta(name, m)
                return self._send(200, {"ok": True, "meta": m})
            if act == "extract" and method == "POST":
                start_extract(name)
                return self._send(200, {"ok": True})
            if act == "job":
                return self._send(200, JOBS.get(name, {"state": "idle"}))
            if act == "concept":
                if method == "POST":
                    b = self._json()
                    con = concept(name)
                    for k, v in (b.get("set") or {}).items():
                        con.set(k, v)
                    for p in b.get("packages") or []:
                        con.set_package(p["tag"], p.get("conn"), p.get("qty"), dwg=p.get("dwg"), name=p.get("title"))
                    if b.get("reset"):
                        for it in con.items.values():
                            it["value"] = it["default"]
                    con.save()
                return self._send(200, concept_view(name))
            if act == "summary":
                return self._send(200, summary_view(name))
            if act == "impact":
                ex = extracted(name)
                if not ex:
                    return self._send(200, {"items": []})
                return self._send(200, impact(ex[1], ex[2], concept(name)))
            if act == "scenario" and method == "POST":
                b = self._json()
                s = summary_view(name)
                m = meta(name)
                m.setdefault("scenarios", [])
                m["scenarios"] = [x for x in m["scenarios"] if x["name"] != b["name"]]
                m["scenarios"].append({"name": b["name"], "hw": s["hw"], "comm": s["comm"], "rows": s["rows"],
                                       "cards": s["cards_total"], "per_origin": s["per_origin"],
                                       "changed": [list(x) for x in concept(name).changed()],
                                       "saved": time.strftime("%m-%d %H:%M")})
                save_meta(name, m)
                return self._send(200, m["scenarios"])
            if act == "review":
                return self._send(200, review_view(name, q))
            if act == "page":
                f = page_png(name, int(parts[4]))
                return self._send(200, f.read_bytes(), "image/png")
            if act == "export" and method == "POST":
                return self._send(200, export(name, self._json()))
            if act == "outputs":
                d = pdir(name) / "output"
                fl = sorted(d.iterdir(), key=lambda p: -p.stat().st_mtime) if d.exists() else []
                return self._send(200, [{"name": f.name, "size": f.stat().st_size,
                                         "time": time.strftime("%m-%d %H:%M", time.localtime(f.stat().st_mtime))} for f in fl])
            if act == "file":
                f = (pdir(name) / "output" / Path(parts[4]).name)
                if not f.exists():
                    return self._send(404, {"error": "no file"})
                ctype = "application/pdf" if f.suffix == ".pdf" else \
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                from urllib.parse import quote
                return self._send(200, f.read_bytes(), ctype,
                                  {"Content-Disposition": f"attachment; filename*=UTF-8''{quote(f.name)}"})
            return self._send(404, {"error": "unknown"})
        except Exception as e:
            return self._send(400, {"error": str(e)})

    def do_GET(self):
        self._route("GET")

    def do_POST(self):
        self._route("POST")


def main(port: int = 8765, open_browser: bool = True):
    HOME.mkdir(parents=True, exist_ok=True)
    srv = ThreadingHTTPServer(("127.0.0.1", port), H)
    url = f"http://127.0.0.1:{port}/"
    print(f"계장 개념설계 IO 산출 - {url}  (프로젝트 폴더: {HOME})  종료: Ctrl+C")
    if open_browser:
        threading.Timer(0.8, lambda: webbrowser.open(url)).start()
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
