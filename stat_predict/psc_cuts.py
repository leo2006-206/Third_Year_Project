"""PSC cut table: for every request kind (servers_benchmark/request_kind.txt + syn_video/request_kind_syn.txt) and every
valid cut k, which request tokens form the bottom sub segment (roots 0..k-1) and its sub id (the PSC key without the segment).
The main server has no JSON parser, so the show specs' layer graphs are flattened here (like features.csv).

Root rule = features.rendered_layers / Dana createOffloadNode: a source adds a root, a children-1 transform joins the last
root, a children-0 transform merges all roots before it. Prefix tokens are matched BY POSITION (forest720 has three
'video_alpha' layers): walk the prefix layers in spec order, each takes the next unused request token with its name.
Valid cut: 1 <= k < min(n, roots) (k >= roots or n > roots makes Dana hang, kb/partial_segment.md 16).

Output stat_predict/psc_cuts.csv, one row per (kind, k):
  rest        path after t_end, as in request_kind.txt (raw '|'), e.g. 1280/720/race/landscape/4/race|race/...
  k, n, roots cut, the request's count token, roots of the variant
  prefix_pos  ';'-joined indices into the layer tokens of rest (the tokens after the count token) that belong to the sub
  one_to_one  1 = request size == spec variant size (pair full exact; 0 -> Dana's surface fallback, 37 dB -> never pair)
  sub_id      show/w/h/variant/style/k/<sorted prefix tokens joined by '/'>  (PSC key = sub_id + segment)
  show        show file
Run: python3 stat_predict/psc_cuts.py
"""
import csv
from pathlib import Path

from features import load_json

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
KIND_FILES = [ROOT / "servers_benchmark/request_kind.txt", ROOT / "syn_video/request_kind_syn.txt"]


def roots(layers: list) -> list[list[int]]:
    """[Honest] root nodes as lists of spec layer positions (same rule as features.rendered_layers)."""
    graph: list[list[int]] = []
    for i, layer in enumerate(layers):
        if layer["type"] == "transform":
            if layer.get("children", 0) == 1 and graph:
                graph[-1].append(i)
            else:
                graph = [[j for node in graph for j in node] + [i]]
        else:
            graph.append([i])
    return graph


def variant_of(spec: dict, name: str, style: str) -> dict:
    """[Honest] Experience:loadVariant match: same name, and same style when the variant has one."""
    for v in spec["variants"]:
        if v["name"] == name and (v.get("style") is None or v.get("style") == style):
            return v
    raise KeyError(f"variant {name}/{style}")


def prefix_positions(layers: list, graph: list[list[int]], tokens: list[str], k: int) -> list[int]:
    """[Honest] indices of the request tokens of the layers in roots 0..k-1, matched by position."""
    used: list[int] = []
    for j in sorted(j for node in graph[:k] for j in node):
        for t_i, t in enumerate(tokens):
            if t_i not in used and t.split("|")[0] == layers[j]["name"]:
                used.append(t_i)
                break
    return sorted(used)


def cut_rows(kind: str) -> list[dict]:
    """[Dishonest I/O: reads the spec] rows of one request kind."""
    p = kind.split("/")
    show, w, h, var, style, n = p[3], int(p[6]), int(p[7]), p[8], p[9], int(p[10])
    tokens = p[11:]
    v = variant_of(load_json(f"shows/{show}"), var, style)
    graph = roots(v["layers"])
    one = int(v.get("width") == w and v.get("height") == h)
    rows = []
    for k in range(1, min(n, len(graph))):
        pos = prefix_positions(v["layers"], graph, tokens, k)
        sub_id = "/".join([show, str(w), str(h), var, style, str(k)] + sorted(tokens[i] for i in pos))
        rows.append(dict(rest="/".join(p[6:]), k=k, n=n, roots=len(graph), prefix_pos=";".join(map(str, pos)),
                         one_to_one=one, sub_id=sub_id, show=show))
    return rows


def main():
    kinds = [l.strip() for f in KIND_FILES for l in f.read_text().splitlines() if l.strip().startswith("/offload")]
    rows = [r for kind in kinds for r in cut_rows(kind)]
    out = HERE / "psc_cuts.csv"
    with out.open("w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    subs = {r["sub_id"] for r in rows}
    no_cut = len(kinds) - len({(r["show"], r["rest"]) for r in rows})  # rests repeat across shows (f1 / f1_full)
    print(f"{out.relative_to(ROOT)}: {len(rows)} rows (kind x cut), {len(kinds)} kinds ({no_cut} with no cut), "
          f"{len(subs)} distinct sub ids, not 1:1 rows {sum(1 for r in rows if not r['one_to_one'])}, "
          f"{out.stat().st_size / 1024:.0f} KiB")
    # sharing: how many kinds share each sub id (what PSC can reuse across viewers), per show
    by_show: dict[str, list[int]] = {}
    count: dict[str, int] = {}
    for r in rows:
        count[r["sub_id"]] = count.get(r["sub_id"], 0) + 1
    for sid, c in count.items():
        by_show.setdefault(sid.split("/")[0], []).append(c)
    print(f"{'show':24} {'sub ids':>7} {'shared by >=2 kinds':>19} {'max kinds/sub':>13}")
    for show, cs in sorted(by_show.items()):
        print(f"{show:24} {len(cs):7} {sum(1 for c in cs if c >= 2):19} {max(cs):13}")


if __name__ == "__main__":
    main()
