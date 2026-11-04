"""点数計算の突き合わせ用テストケースを作る（設計書 8章。開発時のみ使用、配布物には含めない）。

Python の `mahjong` ライブラリ（MIT、天鳳の大量の牌譜で検証済み）でランダムなアガリ手を計算し、
期待値を JSON に書き出す。Rust 側の crates/core/tests/scoring_crosscheck.rs がこれを読んで照合する。

    pip install mahjong==2.1.0
    python tools/gen_scoring_cases.py --count 600 --out crates/core/tests/data/mahjong_cases.json

ルールは janscope-core の GameConfig::preset_standard() に合わせる
（喰いタンあり・赤3・ダブル役満あり・数え役満あり・切り上げなし・連風牌の雀頭4符）。
"""

from __future__ import annotations

import argparse
import json
import random
from pathlib import Path

from mahjong.constants import EAST
from mahjong.hand_calculating.hand import HandCalculator
from mahjong.hand_calculating.hand_config import HandConfig, OptionalRules
from mahjong.meld import Meld

SUITS = "mpsz"


def name(kind: int, red: bool = False) -> str:
    if kind >= 27:
        return f"{kind - 27 + 1}z"
    return f"{0 if red else kind % 9 + 1}{SUITS[kind // 9]}"


class Pool:
    """136枚の牌の id を払い出す（赤5は各色の copy 0 = mahjong ライブラリの 16/52/88）。"""

    def __init__(self) -> None:
        self.used = [0] * 34
        self.red_used = [False] * 3

    def can(self, kind: int, n: int = 1) -> bool:
        return self.used[kind] + n <= 4

    def take(self, kind: int, want_red: bool) -> tuple[int, bool]:
        assert self.used[kind] < 4
        is_five = kind < 27 and kind % 9 == 4
        suit = kind // 9
        if is_five and want_red and not self.red_used[suit]:
            self.red_used[suit] = True
            self.used[kind] += 1
            return kind * 4, True
        # 赤以外の copy（1..3）。赤が未使用で copy が足りなければ copy 0 を通常牌として使うことはしない
        copies = [1, 2, 3] if is_five else [0, 1, 2, 3]
        taken = self.used[kind] - (1 if is_five and self.red_used[suit] else 0)
        if taken >= len(copies):
            # 赤しか残っていない
            self.red_used[suit] = True
            self.used[kind] += 1
            return kind * 4, True
        self.used[kind] += 1
        return kind * 4 + copies[taken], False


def random_hand(rng: random.Random):
    pool = Pool()
    groups = []  # (type, [kind...])
    chiitoi = rng.random() < 0.06
    if chiitoi:
        kinds = rng.sample(range(34), 7)
        for k in kinds:
            groups.append(("pair", [k, k]))
    else:
        while len(groups) < 4:
            if rng.random() < 0.6:
                s = rng.randrange(3)
                n = rng.randrange(7)
                ks = [s * 9 + n + i for i in range(3)]
                if all(pool.can(k) for k in ks):
                    for k in ks:
                        pool.used[k] += 1
                    groups.append(("seq", ks))
            else:
                k = rng.randrange(34)
                if pool.can(k, 3):
                    pool.used[k] += 3
                    groups.append(("tri", [k] * 3))
        while True:
            k = rng.randrange(34)
            if pool.can(k, 2):
                pool.used[k] += 2
                groups.append(("pair", [k, k]))
                break
    # 実際の牌の id を割り当て直す
    counts = pool.used
    pool = Pool()
    want_red = rng.random() < 0.5
    out_groups = []
    for t, ks in groups:
        out_groups.append((t, [pool.take(k, want_red) for k in ks]))
    del counts

    n_melds = 0 if chiitoi else rng.choice([0, 0, 0, 1, 1, 2, 3])
    melds, closed = [], []
    for t, tiles in out_groups:
        if t != "pair" and len(melds) < n_melds:
            melds.append((t, tiles))
        else:
            closed.extend(tiles)
    return closed, melds


def to_names(tiles):
    return [name(tid // 4, red) for tid, red in tiles]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=600)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    rng = random.Random(args.seed)
    calc = HandCalculator()
    rules = OptionalRules(
        has_open_tanyao=True,
        has_aka_dora=True,
        has_double_yakuman=True,
        kazoe_limit=0,  # 13翻以上は数え役満
        kiriage=False,
        fu_for_open_pinfu=True,
        fu_for_pinfu_tsumo=False,
    )
    cases = []
    while len(cases) < args.count:
        closed, melds = random_hand(rng)
        win = rng.choice(closed)
        tsumo = rng.random() < 0.4
        riichi = not melds and rng.random() < 0.5
        player_wind = rng.randrange(4)
        round_wind = rng.randrange(2)
        dora_ind = rng.randrange(34)
        all_ids = [tid for tid, _ in closed] + [tid for _, ts in melds for tid, _ in ts]
        py_melds = []
        for t, ts in melds:
            ids = [tid for tid, _ in ts]
            py_melds.append(Meld(Meld.CHI if t == "seq" else Meld.PON, ids, opened=True))
        cfg = HandConfig(
            is_tsumo=tsumo, is_riichi=riichi, player_wind=EAST + player_wind, round_wind=EAST + round_wind, options=rules
        )
        res = calc.estimate_hand_value(all_ids, win[0], melds=py_melds, dora_indicators=[dora_ind * 4 + 3], config=cfg)
        case = {
            "closed": to_names(closed),
            "melds": [{"kind": "Chi" if t == "seq" else "Pon", "tiles": to_names(ts)} for t, ts in melds],
            "win": name(win[0] // 4, win[1]),
            "tsumo": tsumo,
            "riichi": riichi,
            "seat_wind": player_wind,
            "round_wind": round_wind,
            "dora_indicators": [name(dora_ind)],
        }
        if res.error:
            if res.error != "no_yaku":
                continue  # 想定外のエラー（生成側の問題）は使わない
            case["expected"] = {"error": "no_yaku"}
        else:
            yakuman = any(y.is_yakuman for y in res.yaku)
            case["expected"] = {
                "han": res.han,
                "fu": res.fu,
                "yakuman": yakuman,
                "main": res.cost["main"],
                "additional": res.cost["additional"],
                "yaku": [str(y) for y in res.yaku],
            }
        cases.append(case)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(cases, ensure_ascii=False, indent=1), encoding="utf-8")
    n_err = sum(1 for c in cases if "error" in c["expected"])
    print(f"{len(cases)} 件（役なし {n_err} 件）→ {args.out}")


if __name__ == "__main__":
    main()
