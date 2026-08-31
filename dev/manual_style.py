"""マニュアル PDF の体裁 (フォント・スタイル・部品)。"""
from __future__ import annotations

import os

from reportlab.lib import colors
from reportlab.lib.enums import TA_CENTER, TA_LEFT
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle
from reportlab.lib.units import mm
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.platypus import (Flowable, KeepTogether, Paragraph, Spacer,
                                Table, TableStyle)

HERE = os.path.dirname(os.path.abspath(__file__))
FONT_DIR = os.path.join(HERE, "fonts")

JP = "NotoJP"
JPB = "NotoJP-Bold"
# コード用は等幅の HackGen (SIL OFL / 埋め込み制限なし)。
# 日本語も等幅なので、コマンドの桁がきれいにそろう。
CODE = "HackGen"
CODE_B = "HackGen-Bold"
MONO = CODE

# 配色 — 夜空をイメージした落ち着いた寒色
INK = colors.HexColor("#14181F")        # 本文
MUTED = colors.HexColor("#5A6472")      # 補足
ACCENT = colors.HexColor("#1F5C8B")     # 見出し
ACCENT_LT = colors.HexColor("#E8F0F6")  # 見出し背景
RULE = colors.HexColor("#C9D4DE")
CODE_BG = colors.HexColor("#F4F6F8")
WARN_BG = colors.HexColor("#FDF3E3")
WARN_LN = colors.HexColor("#D89B2B")
NOTE_BG = colors.HexColor("#EEF6EE")
NOTE_LN = colors.HexColor("#5B9E5B")

PAGE_W, PAGE_H = A4
MARGIN = 20 * mm
CONTENT_W = PAGE_W - 2 * MARGIN


def register_fonts():
    pdfmetrics.registerFont(TTFont(JP, os.path.join(FONT_DIR, "NotoSansJP-Regular.ttf")))
    pdfmetrics.registerFont(TTFont(JPB, os.path.join(FONT_DIR, "NotoSansJP-Bold.ttf")))
    pdfmetrics.registerFontFamily(JP, normal=JP, bold=JPB, italic=JP, boldItalic=JPB)
    pdfmetrics.registerFont(TTFont(CODE, os.path.join(FONT_DIR, "HackGen-Regular.ttf")))
    pdfmetrics.registerFont(TTFont(CODE_B, os.path.join(FONT_DIR, "HackGen-Bold.ttf")))
    pdfmetrics.registerFontFamily(CODE, normal=CODE, bold=CODE_B,
                                  italic=CODE, boldItalic=CODE_B)


def _p(name, size, leading, **kw):
    kw.setdefault("fontName", JP)
    kw.setdefault("textColor", INK)
    kw.setdefault("wordWrap", "CJK")     # 日本語は空白で折り返せないので必須
    return ParagraphStyle(name, fontSize=size, leading=leading, **kw)


class S:
    """スタイル一式。register_fonts() の後に組み立てる。"""

    def __init__(self):
        self.title = _p("title", 26, 36, fontName=JPB, textColor=ACCENT,
                        alignment=TA_CENTER)
        self.subtitle = _p("subtitle", 13, 20, textColor=MUTED,
                           alignment=TA_CENTER)
        self.h1 = _p("h1", 16, 22, fontName=JPB, textColor=colors.white,
                     spaceBefore=0, spaceAfter=0, leftIndent=6, firstLineIndent=0)
        self.h2 = _p("h2", 12.5, 18, fontName=JPB, textColor=ACCENT,
                     spaceBefore=10, spaceAfter=4)
        self.h3 = _p("h3", 11, 16, fontName=JPB, textColor=INK,
                     spaceBefore=8, spaceAfter=2)
        self.body = _p("body", 9.8, 16, spaceAfter=5)
        self.small = _p("small", 8.6, 13.5, textColor=MUTED, spaceAfter=4)
        self.bullet = _p("bullet", 9.8, 15.5, leftIndent=13,
                         firstLineIndent=-11, spaceAfter=2.5)
        self.step = _p("step", 9.8, 15.5, leftIndent=18,
                       firstLineIndent=-14, spaceAfter=3.5)
        self.code = ParagraphStyle("code", fontName=MONO, fontSize=8.6,
                                   leading=13, textColor=INK, wordWrap=None,
                                   splitLongWords=0)
        self.codejp = _p("codejp", 8.6, 13, wordWrap=None, splitLongWords=0)
        self.cap = _p("cap", 8.3, 12, textColor=MUTED, alignment=TA_CENTER,
                      spaceBefore=3, spaceAfter=8)
        self.th = _p("th", 8.8, 12.5, fontName=JPB, textColor=colors.white)
        self.td = _p("td", 8.8, 12.5)
        # 設定名など、途中で改行されると困る文字列用
        self.tdid = ParagraphStyle("tdid", fontName=MONO, fontSize=8.2,
                                   leading=12, textColor=INK, wordWrap=None,
                                   splitLongWords=0)
        self.tdc = _p("tdc", 8.8, 12.5, alignment=TA_CENTER)
        self.foot = _p("foot", 8, 11, textColor=MUTED, alignment=TA_CENTER)


class HRule(Flowable):
    def __init__(self, width, color=RULE, thickness=0.6, space=4):
        super().__init__()
        self.width, self.color, self.thickness, self.space = width, color, thickness, space
        self.height = space * 2

    def draw(self):
        self.canv.setStrokeColor(self.color)
        self.canv.setLineWidth(self.thickness)
        self.canv.line(0, self.space, self.width, self.space)


class TocMark(Flowable):
    """
    目次に載せる項目の目印。高さ 0 なので紙面には何も出ない。
    doc.afterFlowable がこれを見つけて、そのときのページ番号を目次へ渡す。
    """

    def __init__(self, level, label):
        super().__init__()
        self.level, self.label = level, label
        self.width = self.height = 0

    def draw(self):
        pass


def h1(text, st, num=None, toc=True):
    """章見出し — 帯付き。toc=False なら目次には載せない。"""
    label = f"{num}　{text}" if num else text
    t = Table([[Paragraph(label, st.h1)]], colWidths=[CONTENT_W], rowHeights=[9 * mm])
    t.setStyle(TableStyle([
        ("BACKGROUND", (0, 0), (-1, -1), ACCENT),
        ("VALIGN", (0, 0), (-1, -1), "MIDDLE"),
        ("LEFTPADDING", (0, 0), (-1, -1), 8),
        ("TOPPADDING", (0, 0), (-1, -1), 0),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 0),
    ]))
    head = [Spacer(1, 6)]
    if toc:
        head.append(TocMark(0, label))
    return head + [t, Spacer(1, 7)]


def h2(text, st, num=None):
    """節見出し。目次にも載せる。"""
    label = f"{num}　{text}" if num else text
    return [TocMark(1, label), Paragraph(label, st.h2)]


SPACE_DOT = colors.HexColor("#A9B6C2")     # 半角スペースの目印
FULLWIDTH_BOX = colors.HexColor("#E0A33A")  # 全角スペース (打ち間違いのもと) の目印


class CodeBox(Flowable):
    """
    コマンドやコードの枠。

    ・等幅の HackGen で組む (日本語も等幅で、桁がそろう)
    ・スペースの位置に薄い点を打って、いくつ空いているか目で数えられるようにする。
      点は図形として描くので、PDF の文字としては本物の空白のまま。
      読者がコピーして貼り付けても、そのまま動く。
    ・全角スペースは打ち間違いのもとなので、オレンジの枠で目立たせる。
    ・1 行が枠に収まらないときは、折り返さずに文字を小さくする
      (コマンドが途中で折り返されると、読んだ人がそのまま打ち間違える)
    """

    PAD_X = 8
    PAD_Y = 5

    def __init__(self, lines, width=CONTENT_W, size=8.6, font=None):
        super().__init__()
        self.lines = list(lines) or [""]
        self.width = width
        self.font = font or CODE
        inner = width - 2 * self.PAD_X
        widest = max(pdfmetrics.stringWidth(ln, self.font, size)
                     for ln in self.lines)
        if widest > inner:
            size = max(5.8, size * inner / widest)
        self.size = size
        self.lh = size * 1.55
        self.height = 2 * self.PAD_Y + self.lh * len(self.lines)

    def wrap(self, availWidth, availHeight):
        return self.width, self.height

    def draw(self):
        c = self.canv
        c.saveState()
        c.setFillColor(CODE_BG)
        c.setStrokeColor(RULE)
        c.setLineWidth(0.6)
        c.rect(0, 0, self.width, self.height, stroke=1, fill=1)

        c.setFont(self.font, self.size)
        for i, line in enumerate(self.lines):
            base_y = self.height - self.PAD_Y - self.lh * (i + 1) + self.lh * 0.28
            x0 = self.PAD_X
            c.setFillColor(INK)
            c.drawString(x0, base_y, line)          # 本物の文字 (空白も本物)

            for j, ch in enumerate(line):
                if ch not in (" ", "　"):
                    continue
                left = x0 + pdfmetrics.stringWidth(line[:j], self.font, self.size)
                w = pdfmetrics.stringWidth(ch, self.font, self.size)
                if ch == " ":
                    c.setFillColor(SPACE_DOT)
                    c.circle(left + w / 2, base_y + self.size * 0.26,
                             max(self.size * 0.055, 0.42), stroke=0, fill=1)
                else:                                # 全角スペース
                    c.setStrokeColor(FULLWIDTH_BOX)
                    c.setLineWidth(0.5)
                    c.rect(left + 0.6, base_y - self.size * 0.06,
                           w - 1.2, self.size * 0.62, stroke=1, fill=0)
        c.restoreState()


def code_block(lines, st, jp=None, size=8.6):
    """CodeBox を、前後の余白ごと 1 かたまりにして返す。"""
    return [KeepTogether([Spacer(1, 3), CodeBox(lines, size=size), Spacer(1, 6)])]


def callout(title, lines, st, kind="warn"):
    """注意書き / 補足の箱。"""
    bg, ln = (WARN_BG, WARN_LN) if kind == "warn" else (NOTE_BG, NOTE_LN)
    inner = [Paragraph(f"<b>{title}</b>", ParagraphStyle(
        "cot", parent=st.body, fontName=JPB, textColor=ln, spaceAfter=3))]
    for x in lines:
        inner.append(Paragraph(x, st.body))
    inner[-1].style = ParagraphStyle("colast", parent=st.body, spaceAfter=0)
    t = Table([[inner]], colWidths=[CONTENT_W])
    t.setStyle(TableStyle([
        ("BACKGROUND", (0, 0), (-1, -1), bg),
        ("LINEBEFORE", (0, 0), (0, -1), 3, ln),
        ("LEFTPADDING", (0, 0), (-1, -1), 10),
        ("RIGHTPADDING", (0, 0), (-1, -1), 10),
        ("TOPPADDING", (0, 0), (-1, -1), 7),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 7),
    ]))
    return [KeepTogether([Spacer(1, 4), t, Spacer(1, 8)])]


def table(rows, st, widths=None, align=None, header=True):
    """
    1 行目を見出しにした表。
    align は列ごとに None / "c"(中央) / "id"(識別子。途中で改行しない) を指定する。
    """
    data = []
    for i, row in enumerate(rows):
        style = st.th if (header and i == 0) else st.td
        cells = []
        for j, c in enumerate(row):
            s = style
            if not (header and i == 0) and align:
                if align[j] == "c":
                    s = st.tdc
                elif align[j] == "id":
                    s = st.tdid
            cells.append(Paragraph(str(c), s))
        data.append(cells)
    if widths is None:
        widths = [CONTENT_W / len(rows[0])] * len(rows[0])
    t = Table(data, colWidths=widths, repeatRows=1 if header else 0)
    cmds = [
        ("VALIGN", (0, 0), (-1, -1), "MIDDLE"),
        ("LEFTPADDING", (0, 0), (-1, -1), 6),
        ("RIGHTPADDING", (0, 0), (-1, -1), 6),
        ("TOPPADDING", (0, 0), (-1, -1), 4),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 4),
        ("LINEBELOW", (0, 0), (-1, -1), 0.4, RULE),
        ("BOX", (0, 0), (-1, -1), 0.6, RULE),
    ]
    if header:
        cmds += [("BACKGROUND", (0, 0), (-1, 0), ACCENT),
                 ("ROWBACKGROUNDS", (0, 1), (-1, -1),
                  [colors.white, colors.HexColor("#F7FAFC")])]
        # 見出しが 1 列目だけのときは横につなげる (途中で折り返さないように)
        if all(str(c).strip() == "" for c in rows[0][1:]):
            cmds.append(("SPAN", (0, 0), (-1, 0)))
    t.setStyle(TableStyle(cmds))
    return [KeepTogether([Spacer(1, 3), t, Spacer(1, 8)])]


def bullets(items, st, marker="・"):
    return [Paragraph(f"{marker}{x}", st.bullet) for x in items]


def steps(items, st, start=1):
    """番号つき手順。途中にコード枠を挟むときは start= で続きから振る。"""
    return [Paragraph(f"<b>{i}.</b>　{x}", st.step)
            for i, x in enumerate(items, start)]
