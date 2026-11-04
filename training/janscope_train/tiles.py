"""牌の種類（分類クラス）の定義。

クラスは設計書 4.10 の「34種＋赤5＋裏向き」= 38 クラス。
名前は天鳳/MJAI系の表記に合わせた短い形式を正とする。

    1m..9m  萬子      (id 0..8)
    1p..9p  筒子      (id 9..17)
    1s..9s  索子      (id 18..26)
    1z..7z  字牌 東南西北白發中 (id 27..33)
    0m 0p 0s 赤5      (id 34..36)
    back    裏向き     (id 37)

注釈付けツール（Label Studio / CVAT）から書き出したラベルはツールごとにクラスの並びが違うため、
名前で正規の id に変換する（`canonical_id`）。いくつかの別名も受け付ける。
"""

from __future__ import annotations

SUITS = ("m", "p", "s")
HONOR_JA = ("東", "南", "西", "北", "白", "發", "中")

CLASS_NAMES: tuple[str, ...] = (
    *(f"{n}{s}" for s in SUITS for n in range(1, 10)),
    *(f"{n}z" for n in range(1, 8)),
    "0m",
    "0p",
    "0s",
    "back",
)
NUM_CLASSES = len(CLASS_NAMES)
BACK_ID = CLASS_NAMES.index("back")
NAME_TO_ID = {n: i for i, n in enumerate(CLASS_NAMES)}

# FluffyStuff/riichi-mahjong-tiles のファイル名（拡張子なし）
_FLUFFY_SUIT = {"m": "Man", "p": "Pin", "s": "Sou"}
_FLUFFY_HONOR = ("Ton", "Nan", "Shaa", "Pei", "Haku", "Hatsu", "Chun")


def fluffy_name(name: str) -> str:
    """クラス名 → FluffyStuff の SVG ファイル名（拡張子なし）。"""
    if name == "back":
        return "Back"
    num, suit = name[0], name[1]
    if suit == "z":
        return _FLUFFY_HONOR[int(num) - 1]
    if num == "0":
        return f"{_FLUFFY_SUIT[suit]}5-Dora"
    return f"{_FLUFFY_SUIT[suit]}{num}"


def _build_aliases() -> dict[str, int]:
    aliases: dict[str, int] = {}
    for i, n in enumerate(CLASS_NAMES):
        aliases[n.lower()] = i
        aliases[fluffy_name(n).lower()] = i
    for k, ja in enumerate(HONOR_JA):
        aliases[ja] = NAME_TO_ID[f"{k + 1}z"]
    for k, en in enumerate(("east", "south", "west", "north", "white", "green", "red")):
        aliases[en] = NAME_TO_ID[f"{k + 1}z"]
    aliases["発"] = NAME_TO_ID["6z"]
    for s in SUITS:
        aliases[f"5{s}r"] = NAME_TO_ID[f"0{s}"]
        aliases[f"r5{s}"] = NAME_TO_ID[f"0{s}"]
        aliases[f"5{s}-red"] = NAME_TO_ID[f"0{s}"]
    for a in ("ura", "裏", "facedown", "face_down", "hidden"):
        aliases[a] = BACK_ID
    return aliases


_ALIASES = _build_aliases()


def canonical_id(name: str) -> int:
    """注釈ツールのクラス名を正規の id に変換する。未知の名前は KeyError。"""
    key = name.strip().lower()
    if key in _ALIASES:
        return _ALIASES[key]
    raise KeyError(f"未知の牌の名前: {name!r}（使える名前: {', '.join(CLASS_NAMES)}）")


def base_kind(class_id: int) -> int:
    """赤5を通常の5として扱った id（同じ牌は4枚までのチェックなどに使う）。"""
    name = CLASS_NAMES[class_id]
    if name[0] == "0":
        return NAME_TO_ID[f"5{name[1]}"]
    return class_id


def wall_136(red_fives: int = 3) -> list[int]:
    """1組の牌（136枚）。red_fives 枚まで赤5に置き換える（萬→筒→索の順に1枚ずつ）。"""
    tiles: list[int] = []
    for i in range(34):
        tiles += [i] * 4
    for s in SUITS[:red_fives]:
        tiles.remove(NAME_TO_ID[f"5{s}"])
        tiles.append(NAME_TO_ID[f"0{s}"])
    return tiles
