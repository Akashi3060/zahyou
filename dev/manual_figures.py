"""マニュアル用に、その場で描く図。"""
from __future__ import annotations

from reportlab.lib import colors
from reportlab.lib.units import mm
from reportlab.pdfbase import pdfmetrics
from reportlab.platypus import Flowable

from manual_style import (ACCENT, CODE, CODE_BG, INK, JP, JPB, MUTED,
                          RULE)


class NotebookLayout(Flowable):
    """ノートブックが 3 つの短いセルでできていることを示す図。"""

    def __init__(self, width):
        super().__init__()
        self.width = width
        self.height = 88 * mm

    def _cell(self, c, y, h, tag, title, note, lines=3, star=False):
        w = self.width
        x = 14 * mm
        cw = w - x - 4 * mm

        c.setStrokeColor(ACCENT if star else RULE)
        c.setFillColor(colors.white)
        c.setLineWidth(1.4 if star else 0.8)
        c.roundRect(x, y, cw, h, 2 * mm, stroke=1, fill=1)

        # 実行ボタン (▷) — 中を塗らない三角
        c.setStrokeColor(colors.HexColor("#4A5563"))
        c.setLineWidth(0.9)
        p = c.beginPath()
        p.moveTo(x - 8.6 * mm, y + h - 4.6 * mm)
        p.lineTo(x - 8.6 * mm, y + h - 9.0 * mm)
        p.lineTo(x - 4.8 * mm, y + h - 6.8 * mm)
        p.close()
        c.drawPath(p, stroke=1, fill=0)

        c.setFont(JPB, 9)
        c.setFillColor(ACCENT)
        c.drawString(x + 4 * mm, y + h - 6 * mm, tag)
        c.setFont(JPB, 8.6)
        c.setFillColor(INK)
        c.drawString(x + 12 * mm, y + h - 6 * mm, title)
        c.setFont(JP, 7.6)
        c.setFillColor(MUTED)
        c.drawString(x + 12 * mm, y + h - 10.5 * mm, note)

        c.setFillColor(colors.HexColor("#E9EDF1"))
        for i, frac in enumerate((0.72, 0.55, 0.63, 0.40)[:lines]):
            yy = y + h - 15 * mm - i * 3.2 * mm
            if yy > y + 2 * mm:
                c.rect(x + 12 * mm, yy, (cw - 18 * mm) * frac, 1.8 * mm,
                       stroke=0, fill=1)

    def draw(self):
        c = self.canv
        c.saveState()
        h = self.height
        w = self.width

        self._cell(c, h - 20 * mm, 17 * mm, "A", "準備",
                   "エンジンを読み込みます。開いた最初に 1 回だけ", lines=2)
        self._cell(c, h - 39 * mm, 16 * mm, "B", "画像を選ぶ",
                   "ボタンを押してファイルを選びます", lines=1)
        self._cell(c, h - 68 * mm, 26 * mm, "C", "設定して実行",
                   "★ 書き換えるのはここだけ。結果もすぐ下に出ます",
                   lines=4, star=True)

        # 別ファイルのエンジン
        x = 14 * mm
        cw = w - x - 4 * mm
        c.setStrokeColor(RULE)
        c.setFillColor(CODE_BG)
        c.setDash(2, 2)
        c.setLineWidth(0.8)
        c.roundRect(x + cw * 0.34, h - 80 * mm, cw * 0.66, 8.5 * mm, 2 * mm,
                    stroke=1, fill=1)
        c.setDash()
        c.setFont(JPB, 8)
        c.setFillColor(MUTED)
        c.drawString(x + cw * 0.34 + 4 * mm, h - 77.2 * mm, "zahyou_engine.py")
        c.setFont(JP, 7.4)
        c.drawString(x + cw * 0.34 + 33 * mm, h - 77.2 * mm,
                     "解析の中身 (開く必要はありません)")

        # A から engine への矢印
        c.setStrokeColor(colors.HexColor("#9AA7B4"))
        c.setLineWidth(0.8)
        c.setDash(2, 2)
        c.line(x + cw * 0.34 + 6 * mm, h - 71.5 * mm,
               x + cw * 0.34 + 6 * mm, h - 20 * mm + 8 * mm)
        c.setDash()

        c.setFont(JP, 7.6)
        c.setFillColor(MUTED)
        c.drawString(x - 8.6 * mm, 1 * mm,
                     "※ 左端の ▷ が実行ボタンです。"
                     "その右の ∨ は「デバッグ」のメニューで、折りたたみではありません。")
        c.restoreState()


class ArrowLegend(Flowable):
    """出力される図の記号の説明。"""

    def __init__(self, width):
        super().__init__()
        self.width = width
        self.height = 26 * mm

    def draw(self):
        c = self.canv
        c.saveState()
        items = [
            ("cross", "水色の × ", "画像の中心（いま望遠鏡が向いている点）"),
            ("circle", "黄色の ○ ", "目標の天体。画面内にあるときだけ出ます"),
            ("arrow", "赤い矢印 ", "中心から目標へ向かう向き。この向きへ動かします"),
            ("green", "緑の ○ ", "プログラムが星として拾った点"),
        ]
        y = self.height - 5 * mm
        for kind, name, desc in items:
            cx, cy = 6 * mm, y + 1.2 * mm
            if kind == "cross":
                c.setStrokeColor(colors.HexColor("#00B7D8"))
                c.setLineWidth(1.4)
                c.line(cx - 2 * mm, cy - 2 * mm, cx + 2 * mm, cy + 2 * mm)
                c.line(cx - 2 * mm, cy + 2 * mm, cx + 2 * mm, cy - 2 * mm)
            elif kind == "circle":
                c.setStrokeColor(colors.HexColor("#E8C020"))
                c.setLineWidth(1.4)
                c.circle(cx, cy, 2.2 * mm, stroke=1, fill=0)
            elif kind == "green":
                c.setStrokeColor(colors.HexColor("#5FCB5F"))
                c.setLineWidth(1.0)
                c.circle(cx, cy, 2.0 * mm, stroke=1, fill=0)
            else:
                c.setStrokeColor(colors.HexColor("#D8322C"))
                c.setFillColor(colors.HexColor("#D8322C"))
                c.setLineWidth(1.6)
                c.line(cx - 2.6 * mm, cy - 1.4 * mm, cx + 1.4 * mm, cy + 1.4 * mm)
                p = c.beginPath()
                p.moveTo(cx + 2.6 * mm, cy + 2.0 * mm)
                p.lineTo(cx + 0.4 * mm, cy + 1.9 * mm)
                p.lineTo(cx + 1.9 * mm, cy - 0.2 * mm)
                p.close()
                c.drawPath(p, stroke=0, fill=1)
            c.setFont(JPB, 8.4)
            c.setFillColor(INK)
            c.drawString(12 * mm, y, name)
            c.setFont(JP, 8.4)
            c.setFillColor(colors.HexColor("#3A424E"))
            c.drawString(32 * mm, y, desc)
            y -= 6 * mm
        c.restoreState()


class FoldHint(Flowable):
    """行の左はしの ∨ でコードを折りたたむ操作の図。"""

    PW = 92 * mm          # コード枠の幅
    CX = 99 * mm          # 説明文の左はし

    def __init__(self, width):
        super().__init__()
        self.width = width
        self.height = 44 * mm

    def _chevron(self, c, cx, cy, direction):
        """折りたたみの矢印。フォントに頼らず線で描く。"""
        s = 1.3 * mm
        c.setStrokeColor(colors.HexColor("#5A6472"))
        c.setLineWidth(1.0)
        c.setLineCap(1)
        c.setLineJoin(1)
        if direction == "down":
            c.line(cx - s, cy + s * 0.6, cx, cy - s * 0.6)
            c.line(cx, cy - s * 0.6, cx + s, cy + s * 0.6)
        else:
            c.line(cx - s * 0.6, cy + s, cx + s * 0.6, cy)
            c.line(cx + s * 0.6, cy, cx - s * 0.6, cy - s)

    def _panel(self, c, y, h):
        c.setStrokeColor(RULE)
        c.setFillColor(CODE_BG)
        c.setLineWidth(0.7)
        c.roundRect(0, y, self.PW, h, 1.5 * mm, stroke=1, fill=1)

    def _note(self, c, y, lines):
        for i, ln in enumerate(lines):
            c.setFont(JPB if i == 0 else JP, 8.4)
            c.setFillColor(INK if i == 0 else colors.HexColor("#3A424E"))
            c.drawString(self.CX, y - i * 4.6 * mm, ln)

    def draw(self):
        c = self.canv
        c.saveState()
        h = self.height
        size, lh = 7.8, 3.9 * mm

        # ---- 上: 開いた状態 ------------------------------------------
        top_h, top_y = 15 * mm, h - 15 * mm
        self._panel(c, top_y, top_h)
        c.setFont(CODE, size)
        c.setFillColor(INK)
        base = top_y + top_h - 4.6 * mm
        for i, ln in enumerate(["try:",
                                "    with open(_path) as _f:",
                                "        exec(_f.read())"]):
            c.drawString(8 * mm, base - i * lh, ln)
        self._chevron(c, 4.8 * mm, base + 0.9 * mm, "down")
        self._note(c, base + 0.2 * mm,
                   ["① 行の左はしにマウスを置く",
                    "∨ が出てきます。押すと畳めます"])

        # ---- 矢印 ----------------------------------------------------
        c.setStrokeColor(colors.HexColor("#9AA7B4"))
        c.setLineWidth(0.9)
        c.line(6 * mm, top_y - 1.5 * mm, 6 * mm, top_y - 6.0 * mm)
        p = c.beginPath()
        p.moveTo(6 * mm, top_y - 7.4 * mm)
        p.lineTo(4.8 * mm, top_y - 5.6 * mm)
        p.lineTo(7.2 * mm, top_y - 5.6 * mm)
        p.close()
        c.setFillColor(colors.HexColor("#9AA7B4"))
        c.drawPath(p, stroke=0, fill=1)
        c.setFont(JP, 7.8)
        c.setFillColor(MUTED)
        c.drawString(9.5 * mm, top_y - 5.4 * mm, "クリック")

        # ---- 下: 畳んだ状態 ------------------------------------------
        bot_h, bot_y = 8.6 * mm, h - 15 * mm - 9.4 * mm - 8.6 * mm
        self._panel(c, bot_y, bot_h)
        c.setFillColor(colors.HexColor("#E3EAF2"))       # 畳んだ行は色が付く
        c.rect(1 * mm, bot_y + 2.2 * mm, self.PW - 2 * mm, 4.2 * mm,
               stroke=0, fill=1)
        c.setFont(CODE, size)
        c.setFillColor(INK)
        c.drawString(8 * mm, bot_y + 3.3 * mm, "try:")
        c.setFillColor(MUTED)
        c.drawString(8 * mm + pdfmetrics.stringWidth("try: ", CODE, size),
                     bot_y + 3.3 * mm, "⋯")
        self._chevron(c, 4.8 * mm, bot_y + 4.2 * mm, "right")
        self._note(c, bot_y + bot_h - 3.0 * mm,
                   ["② 中身が 1 行に縮みます",
                    "左は ＞ に変わり、行末に ⋯ が付きます"])

        c.setFont(JP, 7.8)
        c.setFillColor(MUTED)
        c.drawString(0, 6.2 * mm,
                     "※ 矢印の上にマウスを置くと"
                     "「クリックして範囲を折りたたみます。」と説明が出ます。"
                     "これが目印です。")
        c.drawString(0, 1.8 * mm,
                     "※ 変わるのは見た目だけです。畳んだまま実行しても、"
                     "動くのは中身の全部です。もう一度押すと開きます。")
        c.restoreState()
