#!/usr/bin/env python
"""すれ違い通信「変な文章生成マシーン」の交換ロジックを PC 上で再現する。

BLE 無し。交換ルールとスロット構成をチームで固めるための試作。

  python sim.py                     # 対話モード
  python sim.py --auto 30 --seed 1  # 30 回ランダムにすれ違わせて結果を出す

交換ルール（Scrapbox の仕様を「受け取る側が選ぶ」形に置き換えたもの）:
  各ホストは「自分の一式 own」（全スロットに 1 語ずつ）と
  「組み立て中の文章 collected」（最初は空）を持つ。
  A と B がすれ違うと、A は自分の collected に無いスロットを 1 つランダムに選び、
  B.own のその語を collected に入れる。B も同様に A から受け取る。
  全スロットが埋まったら文章の完成。以降は受け取らない。
  ※「B が A の足りない部分を選んで渡す」と結果は同じ。名札（advertise）だけで成立する。
"""
from __future__ import annotations

import argparse
import json
import random
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent

# パイプ経由でも UTF-8 で出す（Windows でリダイレクトすると CP932 になって化けるため）
for _s in (sys.stdout, sys.stderr):
    if hasattr(_s, "reconfigure"):
        _s.reconfigure(encoding="utf-8", errors="replace")


class Host:
    def __init__(self, name: str, slots: list[dict], rng: random.Random):
        self.name = name
        self.slots = slots
        self.own: dict[str, str] = {s["key"]: rng.choice(s["words"]) for s in slots}
        self.collected: dict[str, str] = {}

    # ----- 状態 -----
    def missing(self) -> list[str]:
        return [s["key"] for s in self.slots if s["key"] not in self.collected]

    def complete(self) -> bool:
        return not self.missing()

    def sentence(self) -> str:
        return "　".join(self.collected.get(s["key"], "＿＿") for s in self.slots)

    def own_sentence(self) -> str:
        return "　".join(self.own[s["key"]] for s in self.slots)

    # ----- 交換 -----
    def take_from(self, other: "Host", rng: random.Random) -> str | None:
        """other.own から、自分に無いスロットを 1 つ受け取る。受け取ったスロット key を返す。"""
        miss = self.missing()
        if not miss:
            return None
        key = rng.choice(miss)
        self.collected[key] = other.own[key]
        return key

    # ----- 表示 -----
    def describe(self) -> str:
        lines = [f"[{self.name}] 一式: {self.own_sentence()}"]
        for s in self.slots:
            k = s["key"]
            mark = "●" if k in self.collected else "○"
            lines.append(f"  {mark} {s['label']:<6} {self.collected.get(k, '')}")
        status = "完成" if self.complete() else f"あと {len(self.missing())}"
        lines.append(f"  → {self.sentence()}  ({status})")
        return "\n".join(lines)


def meet(a: Host, b: Host, rng: random.Random) -> list[str]:
    """A と B がすれ違う。両方向に交換し、ログ行を返す。"""
    label = {s["key"]: s["label"] for s in a.slots}
    log = []
    for me, you in ((a, b), (b, a)):
        key = me.take_from(you, rng)
        if key is None:
            log.append(f"{me.name} は完成済み（{you.name} から受け取らない）")
        else:
            log.append(f"{me.name} ← {you.name}: {label[key]}「{me.collected[key]}」")
            if me.complete():
                log.append(f"★ {me.name} 完成: {me.sentence()}")
    return log


def load_slots(path: Path) -> list[dict]:
    data = json.loads(path.read_text(encoding="utf-8"))
    slots = data["slots"]
    if not slots:
        sys.exit("words.json に slots がありません")
    for s in slots:
        if not s.get("words"):
            sys.exit(f"スロット {s.get('key')} に語句がありません")
    return slots


def make_hosts(n: int, slots: list[dict], rng: random.Random) -> dict[str, Host]:
    names = [chr(ord("A") + i) for i in range(n)]
    return {nm: Host(nm, slots, rng) for nm in names}


def run_auto(hosts: dict[str, Host], times: int, rng: random.Random) -> None:
    names = list(hosts)
    for i in range(1, times + 1):
        a, b = rng.sample(names, 2)
        for line in meet(hosts[a], hosts[b], rng):
            print(f"#{i:02d} {a}-{b}  {line}")
        if all(h.complete() for h in hosts.values()):
            print(f"全員完成（{i} 回）")
            break
    print()
    for h in hosts.values():
        print(h.describe())
        print()


def repl(hosts: dict[str, Host], slots: list[dict], rng: random.Random) -> None:
    help_text = (
        "コマンド:\n"
        "  hosts            ホスト一覧と完成状況\n"
        "  show A           A の一式と組み立て中の文章\n"
        "  meet A B         A と B がすれ違う\n"
        "  auto N           ランダムに N 回すれ違わせる\n"
        "  words            語句表のスロットと語数\n"
        "  reset            全員の組み立て中の文章を空にする\n"
        "  quit"
    )
    print(help_text)
    while True:
        try:
            raw = input("> ").strip()
        except (EOFError, KeyboardInterrupt):
            print()
            return
        if not raw:
            continue
        cmd, *args = raw.split()
        cmd = cmd.lower()
        try:
            if cmd in ("quit", "exit", "q"):
                return
            elif cmd in ("help", "?"):
                print(help_text)
            elif cmd == "hosts":
                for h in hosts.values():
                    st = "完成" if h.complete() else f"あと{len(h.missing())}"
                    print(f"  {h.name}: {h.sentence()}  ({st})")
            elif cmd == "show":
                print(hosts[args[0].upper()].describe())
            elif cmd == "meet":
                a, b = args[0].upper(), args[1].upper()
                if a == b:
                    print("同じホスト同士はすれ違えない")
                    continue
                for line in meet(hosts[a], hosts[b], rng):
                    print("  " + line)
            elif cmd == "auto":
                run_auto(hosts, int(args[0]) if args else 10, rng)
            elif cmd == "words":
                for s in slots:
                    print(f"  {s['key']:<6} {s['label']:<6} {len(s['words'])} 語")
            elif cmd == "reset":
                for h in hosts.values():
                    h.collected.clear()
                print("リセットした")
            else:
                print("不明なコマンド。help で一覧")
        except (KeyError, IndexError, ValueError) as e:
            print(f"引数が変: {e!r}。help で一覧")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--words", type=Path, default=HERE / "words.json", help="語句表 JSON")
    ap.add_argument("--hosts", type=int, default=6, help="ホスト数（既定 6）")
    ap.add_argument("--seed", type=int, default=None, help="乱数シード（再現用）")
    ap.add_argument("--auto", type=int, default=None, help="対話せずに N 回すれ違わせて終了")
    args = ap.parse_args()

    rng = random.Random(args.seed)
    slots = load_slots(args.words)
    hosts = make_hosts(args.hosts, slots, rng)

    print(f"語句表: {args.words.name}  スロット {len(slots)}  ホスト {len(hosts)}")
    if args.auto is not None:
        run_auto(hosts, args.auto, rng)
    else:
        repl(hosts, slots, rng)


if __name__ == "__main__":
    main()
