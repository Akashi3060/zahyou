"""
天体画像解析プログラム「zahyou」導入・運用マニュアル v3 を PDF で作る。

  python make_manual.py [出力先.pdf]

フォントは Noto Sans JP (SIL Open Font License / 埋め込み制限なし) を埋め込む。
dev/fonts に静的ウェイトが無ければ、Windows の可変フォントから作る。
"""
from __future__ import annotations

import os
import sys

from reportlab.lib import colors
from reportlab.lib.pagesizes import A4
from reportlab.lib.units import mm
from reportlab.platypus import (BaseDocTemplate, Frame, Image, KeepTogether,
                                NextPageTemplate, PageBreak, PageTemplate,
                                Paragraph, Spacer)
from reportlab.platypus.tableofcontents import TableOfContents

import manual_style as ms
from manual_figures import AppWindow, ArrowLegend, FoldHint, NotebookLayout
from manual_style import (ACCENT, CONTENT_W, HRule, INK, JP, JPB, MARGIN,
                          MUTED, PAGE_H, PAGE_W, RULE, S, TocMark, bullets,
                          callout, code_block, h1, h2, steps, table)

HERE = os.path.dirname(os.path.abspath(__file__))
ASSETS = os.path.join(HERE, "assets")
VERSION = "v3"
DATE = "2026年8月"


# ---------------------------------------------------------------- 下ごしらえ ---
#  フォントは 2 種類あわせて 33 MB あり、リポジトリに入れるには重い。
#  どちらも SIL Open Font License / 埋め込み制限なし (fsType=0) なので、
#  ここで手元から用意する。fonts/ は .gitignore してある。
FONT_DIR = os.path.join(HERE, "fonts")


def _ensure_noto():
    """本文用。Windows 同梱の可変フォントから Regular / Bold を切り出す。"""
    if os.path.exists(os.path.join(FONT_DIR, "NotoSansJP-Regular.ttf")):
        return
    from fontTools.ttLib import TTFont as FTFont
    from fontTools.varLib import instancer
    src = r"C:\Windows\Fonts\NotoSansJP-VF.ttf"
    if not os.path.exists(src):
        raise SystemExit(
            "Noto Sans JP が見つかりません。\n"
            "  https://fonts.google.com/noto/specimen/Noto+Sans+JP から入手し、\n"
            f"  {FONT_DIR} に NotoSansJP-Regular.ttf と -Bold.ttf を置いてください。")
    for name, wght in (("NotoSansJP-Regular.ttf", 400),
                       ("NotoSansJP-Bold.ttf", 700)):
        f = FTFont(src)
        instancer.instantiateVariableFont(f, {"wght": wght}, inplace=True,
                                          updateFontNames=True)
        f.save(os.path.join(FONT_DIR, name))
        print(f"  フォントを作成: {name}")


def _ensure_hackgen():
    """コード枠用の等幅。インストール済みならそこからコピーする。"""
    import shutil
    need = ("HackGen-Regular.ttf", "HackGen-Bold.ttf")
    candidates = [os.path.expandvars(r"%LOCALAPPDATA%\Microsoft\Windows\Fonts"),
                  r"C:\Windows\Fonts"]
    for n in need:
        dest = os.path.join(FONT_DIR, n)
        if os.path.exists(dest):
            continue
        src = next((os.path.join(d, n) for d in candidates
                    if os.path.exists(os.path.join(d, n))), None)
        if src is None:
            raise SystemExit(
                f"{n} が見つかりません。\n"
                "  HackGen (白源) を https://github.com/yuru7/HackGen/releases から\n"
                f"  入手してインストールするか、{FONT_DIR} に直接置いてください。")
        shutil.copyfile(src, dest)
        print(f"  フォントをコピー: {n}")


def ensure_fonts():
    os.makedirs(FONT_DIR, exist_ok=True)
    _ensure_noto()
    _ensure_hackgen()


def fig(name, width_mm, caption, st):
    path = os.path.join(ASSETS, name)
    if not os.path.exists(path):
        return [Paragraph(f"（図 {name} は未収録）", st.small)]
    from PIL import Image as PILImage
    with PILImage.open(path) as im:
        w, h = im.size
    w_pt = width_mm * mm
    img = Image(path, width=w_pt, height=w_pt * h / w)
    return [KeepTogether([img, Paragraph(caption, st.cap)])]


# ------------------------------------------------------------------ ページ ---
COVER_BAND = 118 * mm          # 表紙の紺色の帯の高さ


def cover_page(canvas, doc):
    canvas.saveState()
    canvas.setFillColor(ACCENT)
    canvas.rect(0, PAGE_H - COVER_BAND, PAGE_W, COVER_BAND, stroke=0, fill=1)
    canvas.setFillColor(colors.HexColor("#2A6E9E"))
    canvas.circle(PAGE_W - 30 * mm, PAGE_H - 30 * mm, 17 * mm, stroke=0, fill=1)
    canvas.setFillColor(colors.white)
    for x, y, r in ((28, 34, 1.2), (58, 20, 0.8), (92, 42, 1.4), (124, 24, 0.7),
                    (150, 40, 1.0), (172, 18, 0.9), (16, 18, 0.7),
                    (44, 96, 0.9), (108, 104, 1.1), (168, 92, 0.8),
                    (78, 88, 0.7), (140, 110, 0.6)):
        canvas.circle(x * mm, PAGE_H - y * mm, r, stroke=0, fill=1)
    canvas.setFillColor(colors.HexColor("#9AA7B4"))
    canvas.setFont(JP, 8.5)
    canvas.drawCentredString(PAGE_W / 2, 14 * mm,
                             "zahyou — 天体画像から目標への向きを求めるプログラム")
    canvas.restoreState()


class ManualDoc(BaseDocTemplate):
    """見出しを見つけるたびに、そのページ番号を目次へ知らせる。"""

    def afterFlowable(self, flowable):
        if isinstance(flowable, TocMark):
            # 表紙を 0 ページ扱いにしているので、本文の見かけのページ番号に合わせる
            self.notify("TOCEntry", (flowable.level, flowable.label,
                                     self.page - 1))


def body_page(canvas, doc):
    canvas.saveState()
    canvas.setStrokeColor(RULE)
    canvas.setLineWidth(0.5)
    canvas.line(MARGIN, PAGE_H - MARGIN + 5 * mm, PAGE_W - MARGIN,
                PAGE_H - MARGIN + 5 * mm)
    canvas.setFont(JP, 8)
    canvas.setFillColor(MUTED)
    canvas.drawString(MARGIN, PAGE_H - MARGIN + 7 * mm,
                      f"天体画像解析プログラム「zahyou」導入・運用マニュアル {VERSION}")
    canvas.drawRightString(PAGE_W - MARGIN, 12 * mm, f"− {doc.page - 1} −")
    canvas.restoreState()


def build(out_path):
    ensure_fonts()
    ms.register_fonts()
    st = S()

    doc = ManualDoc(out_path, pagesize=A4,
                    leftMargin=MARGIN, rightMargin=MARGIN,
                    topMargin=MARGIN, bottomMargin=18 * mm,
                    title="天体画像解析プログラム「zahyou」導入・運用マニュアル v3",
                    author="zahyou", subject="導入・運用マニュアル")
    frame = Frame(MARGIN, 18 * mm, CONTENT_W, PAGE_H - MARGIN - 18 * mm, id="body")
    doc.addPageTemplates([
        PageTemplate(id="cover", frames=[frame], onPage=cover_page),
        PageTemplate(id="body", frames=[frame], onPage=body_page),
    ])

    f = []          # flowables
    add = f.append
    ext = f.extend

    # ============================================================== 表紙 ===
    add(Spacer(1, 22 * mm))
    add(Paragraph("天体画像解析プログラム", ms.ParagraphStyle(
        "ct", parent=st.subtitle, textColor=colors.HexColor("#BFD8EA"),
        fontSize=13)))
    add(Spacer(1, 4 * mm))
    add(Paragraph("zahyou", ms.ParagraphStyle(
        "cz", parent=st.title, textColor=colors.white, fontSize=46, leading=52)))
    add(Spacer(1, 6 * mm))
    add(Paragraph("導入・運用マニュアル", ms.ParagraphStyle(
        "cm", parent=st.subtitle, textColor=colors.HexColor("#CBE1F0"),
        fontSize=15)))
    add(Spacer(1, 37 * mm))          # ここから下は白地
    add(Paragraph("撮った星の写真から、いま望遠鏡がどこを向いているかを割り出し、"
                  "目標の天体へ<b>どちらへどれだけ動かせばよいか</b>を教えてくれます。",
                  ms.ParagraphStyle("clead", parent=st.body, fontSize=11.5,
                                    leading=20, alignment=1)))
    add(Spacer(1, 5 * mm))
    add(Paragraph("星図と見比べる必要はありません。<br/>"
                  "インターネットが無い山の中でも動きます。",
                  ms.ParagraphStyle("clead2", parent=st.body, fontSize=10,
                                    leading=17, alignment=1, textColor=MUTED)))
    add(Spacer(1, 20 * mm))
    ext(table([
        ["v3 でできるようになったこと", ""],
        ["対応する画角", "4′ 〜 2000′（焦点距離およそ 4800 mm まで）"],
        ["設定", "焦点距離などを書かなくても、読み込むだけで解析できます"],
        ["解析の速さ", "目標の方向を手がかりに使うので、以前より大幅に短縮"],
        ["画面", "exe をダブルクリックするだけ。ノートブック版もあります"],
    ], st, widths=[42 * mm, CONTENT_W - 42 * mm]))
    add(Spacer(1, 16 * mm))
    add(Paragraph(f"{VERSION}　{DATE}", ms.ParagraphStyle(
        "cver", parent=st.small, alignment=1, fontSize=9.5)))
    add(NextPageTemplate("body"))
    add(PageBreak())

    # ============================================================== 目次 ===
    ext(h1("目次", st, toc=False))
    toc = TableOfContents()
    toc.levelStyles = [
        ms.ParagraphStyle("toc0", fontName=JPB, fontSize=10.5, leading=19,
                          textColor=INK, wordWrap="CJK"),
        ms.ParagraphStyle("toc1", fontName=JP, fontSize=9.2, leading=15,
                          textColor=ms.colors.HexColor("#3A424E"),
                          leftIndent=14, wordWrap="CJK"),
    ]
    toc.dotsMinLevel = 0
    add(toc)
    add(PageBreak())

    # =========================================================== 1. 概要 ===
    ext(h1("このプログラムでできること", st, "1"))
    add(Paragraph(
        "天体写真（FITS / JPEG / PNG など）を読み込むと、写っている星の並びから "
        "<b>その写真が空のどこを写したものか</b>を突き止めます。"
        "そのうえで、指定した目標の天体が画面のどちらにあるかを矢印で示し、"
        "<b>赤経・赤緯それぞれに何分角動かせばよいか</b>を数字で出します。", st.body))
    add(Paragraph(
        "星図と写真を見比べて「たぶんこの星だろう」と推測する必要がありません。"
        "はじめて望遠鏡を触る人でも、矢印の向きにコントローラーを操作するだけで"
        "目標を導入できます。日没直後に始まる現象のように、準備の時間が"
        "限られている場面でも役に立ちます。", st.body))

    ext(h2("特徴", st))
    ext(bullets([
        "<b>ハイブリッド解析</b> — インターネットにつながっていれば "
        "Astrometry.net のサーバーを使い、つながっていなければ"
        "自分の PC の中（WSL）だけで解析します。切り替えは自動です。",
        "<b>完全オフライン対応</b> — 電波の届かない山の上でも動きます。",
        "<b>導入量が数字で出る</b> — 「東へ 0.92 分角、北へ 1.29 分角」のように、"
        "赤道儀の軸に沿った移動量が直接わかります。",
        "<b>どんな画像でもそのまま</b> — カラーでも白黒でも、動画から切り出した"
        "1 コマでも、日時が焼き込まれていても構いません。焦点距離の設定も不要です。",
    ], st))

    ext(h2("2 つの入り口 — どちらを使うか", st))
    add(Paragraph("<b>解析の中身はどちらもまったく同じ</b>です。"
                  "はじめての方や、とにかく動かしたい方は"
                  "<b>デスクトップ版</b>をどうぞ。", st.body))
    ext(table([
        ["", "デスクトップ版（exe）", "ノートブック版"],
        ["入れるもの", "<b>zahyou.exe 1 個だけ</b>", "Python + VS Code + ノートブック"],
        ["使い方", "ダブルクリック → 画像を選ぶ → 実行", "セルを上から実行"],
        ["オフラインの用意", "<b>［準備］タブのボタン 1 つ</b>",
         "4.4 〜 4.7 を自分で"],
        ["中身を読む・直す", "できません", "できます"],
        ["この本のどこ", "<b>2 章</b>", "4 章・5 章"],
    ], st, widths=[24 * mm, 58 * mm, CONTENT_W - 82 * mm]))

    ext(fig("result_original.png", 150, "図 1　撮影したときと同じ向きの画像。"
            "水色の × が画面の中心、黄色の ○ が目標、赤い矢印が動かす向き。", st))
    ext(fig("result_northup.png", 132, "図 2　北を上・東を左にそろえ直した画像。"
            "赤道儀の軸の向きと一致するので、こちらの方が操作しやすい人もいます。", st))

    add(PageBreak())

    # =================================================== 2. デスクトップ版 ===
    ext(h1("デスクトップ版（exe）を使う — こちらがおすすめ", st, "2"))
    add(Paragraph("<b>zahyou.exe を落として、ダブルクリックするだけです。</b>"
                  "Python も VS Code もインストールも要りません。"
                  "ファイルは 1 個（102 MB）だけです。", st.body))

    ext(h2("落として起動する", st, "2.1"))
    ext(steps([
        "配布ページ（付録 B）から <b>zahyou.exe</b> をダウンロードします。"
        "置き場所はどこでも構いません（デスクトップでも、USB メモリでも）。",
        "ダブルクリックします。初回の起動に 4 秒ほどかかります。",
        "青い画面（次の囲み）が出たら、<b>「詳細情報」→「実行」</b>を押します。",
    ], st))
    ext(callout("初めて実行するとき、青い警告が出ます（正常です）", [
        "「<b>Windows によって PC が保護されました</b>」と表示され、"
        "「実行」ボタンが見当たりません。"
        "<b>「詳細情報」をクリックすると「実行」ボタンが現れます。</b>",
        "これは zahyou に<b>開発元の署名（コード署名証明書）が付いていない</b>ため"
        "です。証明書は有料で、個人が配るソフトには付いていないことがよくあります。"
        "ダウンロードしたファイルには「別の PC から来た」という印が付くので、"
        "署名が無いと Windows がこの警告を出します。"
        "<b>作った本人の PC ではこの印が付かないため、警告は出ません。</b>",
        "ウイルス対策ソフトが誤検知することもあります"
        "（Python のプログラムを 1 つの exe にまとめているためです）。"
        "その場合は、ウイルス対策ソフト側で zahyou.exe を除外に登録してください。",
        "中身が本物か確かめたい方は、配布ページ（付録 B）に載せている SHA256 と、"
        "手元のファイルのものを見比べてください。"
        "PowerShell で次を打つと出ます。",
    ], st))
    ext(code_block([
        "Get-FileHash .\\zahyou.exe -Algorithm SHA256",
    ], st))

    ext(h2("使い方 — 4 つだけ", st, "2.2"))
    ext(steps([
        "<b>［解析］</b>タブの「参照...」を押して、天体画像を選びます。",
        "目標を指定します。<b>天体名</b>（UCAC4 660-021020 など）か、"
        "<b>赤経・赤緯</b>のどちらかです。",
        "<b>「解析を実行」</b>を押します。",
        "画面の上に出る<b>3 つの数字</b>が答えです。"
        "赤経・赤緯それぞれに何分角動かせばよいかを表しています。",
    ], st))
    add(AppWindow(CONTENT_W))
    add(Paragraph("図 3　デスクトップ版の画面。ふだん触るのは左の列と"
                  "「解析を実行」だけです。", st.cap))
    add(Spacer(1, 3))
    add(Paragraph("インターネットにつながっていれば、ここまでで動きます。"
                  "設定（目標・焦点距離・配色など）は次に開いたときも残ります。",
                  st.body))

    ext(h2("ネットの無いところで使う — ［準備］タブ", st, "2.3"))
    add(Paragraph("<b>「まとめて準備する」を押すだけです。</b>"
                  "足りないものを順に用意します。", st.body))
    ext(bullets([
        "<b>WSL</b>（Windows の中で動く Linux）— たいていはそのまま入ります"
        "（実測 40 秒）。<b>管理者の確認（UAC）が出たときだけ</b>「はい」を押し、"
        "そのあと Windows を再起動してから、もう一度押してください。",
        "<b>astrometry.net</b>（解析エンジン本体）",
        "<b>星図データ</b> — 焦点距離とセンサー横幅を入れて「必要な段を選ぶ」を"
        "押すと、要る段だけに印が付きます。途中で止めても続きから落とし直します。",
        "<b>設定ファイル</b>（/etc/astrometry.cfg）",
    ], st))
    add(Paragraph("一覧表の「総合」が<b>「オフライン解析できます」</b>になれば完了です。"
                  "どの段が要るかは 6 章の早見表でも確かめられます。", st.body))
    ext(callout("画面を切り出している人は、画像を選んでから押してください", [
        "［解析］タブで画像を選んでおくと、<b>その画像の縦横を読んで</b>"
        "短辺のぶんまで含めて段を選びます。"
        "掩蔽観測で縦を切り詰めている場合、これをしないと足りません"
        "（理由は 6 章）。",
    ], st))

    ext(h2("暗いところで使う・目標が分からないとき", st, "2.4"))
    ext(bullets([
        "<b>右下の「暗く」</b>（月の印が付いたボタン）— 画面もタイトルバーも"
        "図の中も黒基調になります。"
        "観測中に目が眩みません。もう一度押すと戻ります。",
        "<b>「指定しない（完全ブラインド）」</b> — 目標を決めずに、"
        "「この画像はどこを向いているか」だけを求めます。"
        "上の 3 つの数字は<b>画像中心の赤経・赤緯・画素スケール</b>に変わります。"
        "目標が画面の外にあるか分からないとき、まずこれで向きを確かめられます。",
        "<b>図の下のボタン</b> — 全体 / 戻る / 進む / 移動 / 拡大 / 画像を保存。"
        "拡大して星の位置を確かめたり、図を PNG で保存できます。",
    ], st))
    add(Paragraph("ここから先（3 章〜5 章）は<b>ノートブック版の説明</b>です。"
                  "exe を使う方は <b>6 章</b>（画角と星図データ）へ進んでください。",
                  st.body))

    add(PageBreak())

    # =========================================================== 3. 準備 ===
    ext(h1("準備するもの", st, "3"))
    ext(table([
        ["項目", "必要なもの", "備考"],
        ["パソコン", "Windows 10 または 11（64bit）",
         "オフライン解析には WSL2 が動く必要があります"],
        ["ディスクの空き", "オンラインのみ：1 GB 程度<br/>"
         "オフラインも使う：<b>15 GB 以上</b>",
         "星図データが大きいためです"],
        ["インターネット", "導入のときだけ必須",
         "運用時はオフラインでも構いません"],
        ["天体画像", "FITS / FIT / JPEG / PNG / TIFF",
         "星が 10 個ほど写っていれば十分です"],
    ], st, widths=[26 * mm, 52 * mm, CONTENT_W - 78 * mm]))

    ext(callout("オンラインだけで使うなら、4.4 以降は飛ばせます", [
        "インターネットにつながる場所でしか使わないのであれば、"
        "WSL と星図データ（合計 15 GB ほど）は要りません。"
        "4.1 〜 4.3 と 4.8 だけ済ませてください。",
        "あとから追加することもできます。",
    ], st, kind="note"))

    add(PageBreak())

    # ==================================================== 3. インストール ===
    ext(h1("インストール手順（ノートブック版）", st, "4"))
    add(Paragraph("上から順に進めてください。一度済ませれば、次からは不要です。",
                  st.body))
    ext(callout("コマンドの読み方", [
        "灰色の枠はそのまま打ち込むコマンドです。枠の中の <b>薄い点（·）は半角スペース</b>を"
        "表しています。点そのものは印刷上の目印なので、打つ必要はありません。"
        "スペースを 1 つ空ける、という意味です。",
        "オレンジの四角は<b>全角スペース</b>です。コマンドの中に全角スペースが入ると"
        "動かないので、出てきたら半角に直してください"
        "（このマニュアルのコマンドには全角スペースは入っていません）。",
        "PDF から直接コピーして貼り付けることもできます。点は文字として入りません。",
    ], st, kind="note"))

    ext(h2("Python を入れる", st, "4.1"))
    ext(steps([
        "<b>python.org/downloads</b> を開き、「Download Python」ボタンから"
        "インストーラーをダウンロードします。",
        "インストーラーを起動したら、<b>最初の画面の下にある "
        "「Add python.exe to PATH」に必ずチェックを入れて</b>ください。"
        "ここを忘れると、あとで Python が見つからないと言われます。",
        "「Install Now」を押して、終わるまで待ちます。",
    ], st))
    add(Paragraph("入ったかどうかは、PowerShell を開いて次を打つと確かめられます。",
                  st.body))
    ext(code_block(["python --version"], st))
    add(Paragraph("<font color='#5A6472'>→ Python 3.11.9 のようにバージョンが"
                  "出れば成功です。</font>", st.small))

    ext(h2("VS Code を入れる", st, "4.2"))
    ext(steps([
        "<b>code.visualstudio.com</b> からインストーラーを落として実行します。"
        "選択肢はすべて既定のままで構いません。",
        "VS Code を起動し、左端の四角が 4 つ並んだアイコン（拡張機能）を押します。",
        "検索欄に <b>Python</b> と入れ、Microsoft 製のものを「インストール」。",
        "同じように <b>Jupyter</b> も入れます。",
    ], st))

    ext(h2("ノートブックと必要なライブラリ", st, "4.3"))
    ext(steps([
        "<b>zahyou_v6.ipynb</b> と <b>zahyou_engine.py</b> の 2 つを、"
        "分かりやすい場所（デスクトップに作った zahyou フォルダーなど）に"
        "<b>そろえて</b>置きます。片方だけでは動きません。",
        "<b>zahyou_v6.ipynb</b> を右クリック →「プログラムから開く」→ VS Code を選びます。",
        "画面右上の「カーネルの選択」を押し、<b>Python 環境</b> →"
        "先ほど入れた Python を選びます。",
        "いちばん上の <b>セル A</b> の中の <b>!pip install</b> で始まる行の "
        "<b>#</b> を消して、セルを実行（左端の ▷ か Shift+Enter）します。"
        "必要なライブラリが入ります。",
        "終わったら <b>#</b> を戻しておきます。次からは実行のたびに"
        "入れ直さずに済みます。",
    ], st))
    ext(code_block([
        "# !pip install astroquery astropy matplotlib Pillow reproject ipywidgets scipy",
        "  ↓ 先頭の # を消す",
        "!pip install astroquery astropy matplotlib Pillow reproject ipywidgets scipy",
    ], st, jp=True))

    add(PageBreak())

    ext(h2("【オフラインで使う人だけ】WSL を入れる", st, "4.4"))
    add(Paragraph("WSL（Windows Subsystem for Linux）は、Windows の中で "
                  "Linux を動かすしくみです。オフライン解析のエンジンがこの上で動きます。",
                  st.body))
    ext(steps([
        "スタートボタンを<b>右クリック</b>し、"
        "「ターミナル<b>（管理者）</b>」を選びます。"
        "「このアプリが変更を加えることを許可しますか？」には「はい」と答えます。",
        "次のコマンドを打って Enter。",
    ], st))
    ext(code_block(["wsl --install"], st))
    ext(steps([
        "終わったら PC を<b>再起動</b>します。",
        "再起動後、自動的に Ubuntu の黒い画面が開きます。"
        "<b>ユーザー名</b>（小文字の英字）と<b>パスワード</b>を決めて入力してください。"
        "パスワードは打っても画面に表示されませんが、ちゃんと入力されています。",
        "「Installation successful!」と出れば完了です。",
    ], st, start=3))
    ext(callout("Ubuntu の設定画面が出てこなかったときは", [
        "再起動しても黒い画面が開かないことがあります。"
        "その場合は <b>root</b> という管理者アカウントだけが作られた状態で、"
        "そのまま使えます（<b>sudo</b> を付けなくてもコマンドが通ります）。"
        "zahyou の動作には影響しません。",
        "あとから自分のユーザーを作りたくなったら、"
        "スタートメニューの <b>Ubuntu</b> を開いて "
        "<b>adduser 好きな名前</b> と打ってください。",
    ], st, kind="note"))
    ext(callout("管理者のターミナルでないと失敗します", [
        "「この操作には管理者権限が必要です」と出たときは、"
        "手順 1 のとおり<b>スタートボタンを右クリックして"
        "「ターミナル（管理者）」</b>から開き直してください。",
        "会社や学校の PC で管理者になれない場合は、"
        "オンライン解析のみでお使いください（4.1〜4.3 と 4.8 だけで動きます）。",
    ], st))

    ext(h2("【同】解析エンジン astrometry.net を入れる", st, "4.5"))
    add(Paragraph("ここから先は、<b>zahyou に同梱の setup_wsl.ps1 を使うと"
                  "4.5 〜 4.7 をまとめて片付けられます</b>。"
                  "PowerShell（管理者でなくて構いません）で次の 2 行を実行してください。",
                  st.body))
    add(Paragraph("1 行目の <b>cd</b> は「作業する場所をそこへ移す」という意味です。"
                  "zahyou_v6.ipynb と dev フォルダーを置いた場所を書きます。"
                  "たとえば<b>デスクトップの zahyou フォルダー</b>に置いたなら、"
                  "次のようになります。", st.body))
    ext(code_block([
        "cd $env:USERPROFILE\\Desktop\\zahyou",
        "powershell -ExecutionPolicy Bypass -File dev\\setup_wsl.ps1 -CopyIndexToWsl",
    ], st))
    add(Paragraph("<font color='#5A6472'>$env:USERPROFILE は "
                  "C:\\Users\\（あなたのユーザー名） の意味です。"
                  "置き場所が分からなくなったら、エクスプローラーでそのフォルダーを開き、"
                  "上のアドレス欄をクリックしてパスをコピーして、cd のうしろに"
                  "貼り付けても構いません。</font>", st.small))
    ext(callout("￥ と \\ は同じ文字です", [
        "日本語環境の Windows では、フォルダーの区切りが <b>￥</b>（円マーク）で"
        "表示されることがあります。英語環境の <b>\\</b>（バックスラッシュ）と"
        "まったく同じ文字で、画面上の見た目が違うだけです。",
        "このマニュアルでは <b>\\</b> と書いていますが、お使いの PC で <b>￥</b> と"
        "出ていてもそのままで正しく、書き直す必要はありません。"
        "キーボードでは、どちらも右下の <b>￥</b> キーで打てます。",
    ], st, kind="note"))
    add(Paragraph("手で進めたい場合は、以下の 4.5 〜 4.7 を順に行ってください。",
                  st.small))
    add(Paragraph("スタートメニューから <b>Ubuntu</b> を開き、次を打ちます。"
                  "パスワードを聞かれたら、4.4 で決めたものを入れてください。", st.body))
    ext(code_block([
        "sudo apt update",
        "sudo apt install astrometry.net -y",
    ], st))

    ext(h2("【同】星図データを置く", st, "4.6"))
    add(Paragraph("解析には「どの星がどこにあるか」を書いた星図データ（index ファイル）"
                  "が要ります。<b>使う望遠鏡の画角によって必要なものが変わります</b>ので、"
                  "6 章の早見表で確かめてから落としてください。", st.body))
    ext(steps([
        "C ドライブの直下に <b>AstrometryData</b> という名前のフォルダーを作ります"
        "（＝ <b>C:\\AstrometryData</b>）。",
        "配布サイト（付録 B）から、必要な zip をダウンロードします。",
        "zip を <b>C:\\ の直下</b>に展開します。"
        "<b>C:\\AstrometryData\\index-5205-00.fits</b> のようになれば正解です。",
    ], st))
    ext(callout("まず何を落とせばよいか迷ったら", [
        "焦点距離が 1200 mm 以下の望遠鏡・レンズなら、"
        "<b>AstrometryData.zip</b>（基本セット）だけで足ります。",
        "それより長い焦点距離を使う場合は、6 章の表を見て "
        "<b>index-5204 / 5203 / 5202</b> を必要な分だけ足してください。",
    ], st, kind="note"))

    ext(h2("【同】設定ファイルを書く", st, "4.7"))
    add(Paragraph("解析エンジンに、星図データの置き場所を教えます。"
                  "PowerShell で次の 1 行を実行してください。", st.body))
    ext(code_block([
        "wsl -u root bash -c \"printf 'inparallel\\ncpulimit 300\\nautoindex\\n"
        "add_path /mnt/c/AstrometryData\\n' > /etc/astrometry.cfg\"",
    ], st))
    ext(callout("Linux 側へコピーする必要は、ふつうありません", [
        "上の設定では、Linux 側から Windows のフォルダー（/mnt/c）を読みに行きます。"
        "この読み出しは Linux 内部（ext4）に比べて <b>10 分の 1 ほどの速さ</b>です"
        "（実測: 96 MB の読み込みが 0.25 秒 対 0.03 秒）。",
        "ただし solve-field が実際に読むのは星図データのごく一部なので、"
        "<b>解析時間はほとんど変わりません</b>。"
        "キャッシュを消したうえで測っても、1 枚あたり約 3 秒でした。",
        "どうしても詰めたい場合だけ、次のようにコピーできます。"
        "そのぶん WSL の仮想ディスクが 10 GB ほど増え、"
        "空けても自動では縮まないことに注意してください。",
    ], st, kind="note"))
    ext(code_block([
        "cp -r /mnt/c/AstrometryData ~/AstrometryData",
        "sudo sed -i \"s#/mnt/c/AstrometryData#$HOME/AstrometryData#\" /etc/astrometry.cfg",
    ], st))

    ext(h2("動作確認", st, "4.8"))
    add(Paragraph("VS Code で zahyou_v6.ipynb を開き、セル A → セル B → セル C の順に"
                  "実行します。手元のテスト画像で結果の図が 2 枚出れば導入完了です。",
                  st.body))
    add(Paragraph("オフライン解析だけを試したいときは、セル C の "
                  "<b>SOLVE_MODE</b> を <b>'OFFLINE'</b> にして実行してください。",
                  st.body))
    ext(callout("どのくらい時間がかかるか（実測）", [
        "Ubuntu 26.04 + 星図データ 253 ファイル（4′〜2000′）を "
        r"C:\AstrometryData に置いた構成で、"
        "968 × 548 画素の画像を <b>約 4 秒</b>で解けました。",
        "同じ画像を FITS・PNG・JPEG・カラー・動画キューブ・上下反転の 6 通りに"
        "変換して試しても、すべて同じ空の場所（ずれ 0.1″ 以内）に解けています。",
        "初めての画角では星図データの読み込みに十数秒かかることがありますが、"
        "2 回目からは速くなります。",
    ], st, kind="note"))

    add(PageBreak())

    # ========================================================= 4. 使い方 ===
    ext(h1("使い方（ノートブック版）", st, "5"))

    ext(h2("画面の構成 — 3 つのセル", st, "5.1"))
    add(Paragraph("ノートブックは 3 つのセル（プログラムのかたまり）でできています。"
                  "どれも短く、全部あわせても 90 行ほどです。"
                  "<b>ふだん触るのはセル C だけ</b>で、結果もそのすぐ下に出ます。",
                  st.body))
    add(Paragraph("解析の中身は <b>zahyou_engine.py</b> という別のファイルに入っていて、"
                  "セル A がそれを読み込みます。"
                  "<b>この 2 つは必ず同じフォルダーに置いてください。</b>", st.body))
    add(NotebookLayout(CONTENT_W))
    add(Spacer(1, 4))

    ext(h2("スクロールを減らすコツ", st, "5.2"))
    add(Paragraph("画像を選ぶセル B と、結果が出るセル C は隣どうしなので、"
                  "行ったり来たりしやすくなっています。"
                  "そのうえで、次を覚えておくと快適です。", st.body))
    ext(bullets([
        "<b>2 回目からはセル C だけ実行する。</b>"
        "セル A（準備）は開いた最初の 1 回だけで十分です。"
        "別の画像に変えるときは、セル B で選び直してからセル C を実行します。",
        "<b>図がたまってきたら「すべての出力のクリア」。</b>"
        "画面上部のツールバーにあります。過去の実行結果がまとめて消え、"
        "画面が一気に短くなります。",
        "<b>「アウトライン」でセルへ飛ぶ。</b>"
        "同じくツールバーにあります。スクロールせずに目的のセルへ移動できます。",
        "<b>読まないところは折りたたむ。</b>"
        "コードの行の左はしにマウスを近づけると <b>∨</b> が出ます。"
        "押すと、その行から下のひとかたまりが 1 行に縮みます。"
        "たとえばセル C の設定を書き終えたら、"
        "セル A を丸ごと畳んでしまえば画面がぐっと短くなります。",
    ], st))
    add(KeepTogether([FoldHint(CONTENT_W), Spacer(1, 3)]))
    ext(callout("折りたたみの ∨ は「コードの左はし」。実行ボタンの右の ∨ ではありません", [
        "セルの左端にある <b>▷</b> が実行ボタンです。"
        "そのすぐ右にある小さな <b>∨</b> を押すと "
        "<b>「[デバッグ] セル」</b>というメニューが出ます。"
        "これは不具合を調べるための機能で、折りたたみではありません。",
        "折りたたむための <b>∨</b> は、ボタンの並びではなく"
        "<b>コードの行そのものの左はし</b>にあります（上の図）。"
        "マウスを置いたときだけ出るので、"
        "普段は見えていなくても心配ありません。",
        "なお「畳んだ状態で配る」しくみは VS Code にないため、"
        "このマニュアルの版では<b>そもそも畳まなくてよいように</b>、"
        "長い部分を zahyou_engine.py へ追い出してあります。",
    ], st))

    ext(h2("セル C の設定項目", st, "5.3"))
    ext(table([
        ["設定", "意味", "既定値"],
        ["INPUT_MODE", "目標を天体名で指定するか（'STAR_NAME'）、"
         "赤経赤緯で指定するか（'COORDS'）、"
         "目標を決めないか（<b>'NONE'</b>＝完全ブラインド。"
         "画像がどこを向いているかだけ求めます）",
         "'STAR_NAME'"],
        ["TARGET_STAR_NAME", "目標の天体名。UCAC4 や TYC などのカタログ名、"
         "一般的な名前が使えます", "—"],
        ["RA_INPUT_STR<br/>DEC_INPUT_STR", "赤経・赤緯で指定するときの座標", "—"],
        ["FOCAL_LENGTH_MM", "望遠鏡の焦点距離［mm］。"
         "<b>書かなくても動きます</b>が、書くとオフライン解析が速く確実になります",
         "None"],
        ["SOLVE_MODE", "'AUTO'（自動）/ 'ONLINE'（ネットのみ）/ "
         "'OFFLINE'（WSL のみ）", "'AUTO'"],
        ["ONLINE_TIMEOUT", "ネット解析を諦めるまでの秒数", "120"],
        ["OFFLINE_TIMEOUT", "オフライン解析にかけてよい合計秒数", "300"],
        ["USE_TARGET_AS_HINT", "目標の方向を探索の手がかりに使うか。"
         "速くなるので通常は True のまま", "True"],
        ["SEARCH_RADIUS_DEG", "手がかりから何度の範囲まで探すか。"
         "導入誤差が大きいときは広げます", "5.0"],
        ["IGNORE_EXISTING_WCS", "画像に既に座標情報が入っていても、"
         "無視して解き直すか", "True"],
        ["SHOW_DETECTED_SOURCES", "拾った星に緑の丸を重ねるか", "True"],
        ["IMAGE_PATH", "ボタンを使わずファイルのパスを直接書くとき", "None"],
    ], st, widths=[41 * mm, CONTENT_W - 41 * mm - 20 * mm, 20 * mm],
        align=["id", None, "c"]))

    ext(callout("天体名はオフラインでも使えます（2 回目から）", [
        "天体名から座標を調べるにはインターネットが要ります。"
        "ただし一度調べた名前は PC の中に記憶されるので、"
        "<b>自宅で一度実行しておけば、山の中でも同じ目標なら名前のまま使えます</b>。",
        "初めての目標をオフラインで扱うときは、"
        "INPUT_MODE を 'COORDS' にして赤経赤緯を直接書いてください。",
    ], st, kind="note"))

    ext(h2("出力の読み方", st, "5.4"))
    add(Paragraph("実行すると、まず解析結果の数値が出て、その下に図が 2 枚出ます。",
                  st.body))
    add(ArrowLegend(CONTENT_W))
    add(Spacer(1, 2))
    add(Paragraph("いちばん大事なのは、図の下に出るこの 3 行です。", st.body))
    ext(code_block([
        "  画像中心から目標までのずれ",
        "    全角距離      : 1.583'  (95.0\")",
        "    赤経(RA) 方向 : 0.918' 東へ",
        "    赤緯(Dec)方向 : 1.290' 北へ",
    ], st))
    add(Paragraph("この例なら、赤経を東へ 0.92 分角、赤緯を北へ 1.29 分角動かせば"
                  "目標が画面の中心に来ます。", st.body))

    ext(h2("観測当日の流れ", st, "5.5"))
    ext(steps([
        "望遠鏡を目標のあたりへ向け、1 枚撮ります。露出は短くて構いません。",
        "セル A のボタンでその画像を選びます。",
        "セル C の目標を確認し、実行します。",
        "出てきた矢印の向きに、表示された分だけコントローラーで動かします。",
        "もう一度撮って 2 〜 4 を繰り返すと、確実に中心へ寄っていきます。",
    ], st))

    add(PageBreak())

    # ==================================================== 5. 画角と星図 ===
    ext(h1("画角と星図データ（焦点距離の早見表）", st, "6"))
    add(Paragraph("星図データは<b>画角ごとに段が分かれて</b>います。"
                  "自分の画角に合う段が無いと、どれだけ待っても解けません。", st.body))
    add(Paragraph("大事なのは<b>「短いほうの辺」の画角</b>です。"
                  "プログラムは星が作る四角形の形を星図と突き合わせますが、"
                  "その四角形は<b>短いほうの辺に収まる大きさまでしか作れない</b>ため、"
                  "必要な段は短辺で決まります。"
                  "<b>切り出していない普通の画像でも同じです。</b>", st.body))
    add(Paragraph("画角は次の式で求まります。", st.body))
    ext(code_block([
        "画素スケール[\"/px] = 206.265 x 画素サイズ[um] / 焦点距離[mm]",
        "画角[']            = 画像の横幅[px] x 画素スケール[\"/px] / 60",
        "短辺の画角[']       = 画像の高さ[px] x 画素スケール[\"/px] / 60",
    ], st))
    add(Paragraph("下の表は ZWO ASI290MM（1936 x 1096 画素・画素 2.9 µm）で、"
                  "<b>短辺（1096 画素）の画角</b>から引いた目安です。"
                  "ビニングしても画角は変わりません。", st.small))
    ext(table([
        ["星図データ", "対応する画角", "ASI290MM 全画面での焦点距離", "サイズ"],
        ["index-4107 〜 4119", "22′ 〜 2000′", "〜 500 mm", "364 MB"],
        ["index-5206", "16′ 〜 22′", "500 〜 680 mm", "308 MB"],
        ["index-5205", "11′ 〜 16′", "680 〜 990 mm", "615 MB"],
        ["index-5204", "8′ 〜 11′", "990 〜 1360 mm", "1.2 GB"],
        ["index-5203", "5.6′ 〜 8′", "1360 〜 1950 mm", "2.3 GB"],
        ["index-5202", "4′ 〜 5.6′", "1950 〜 2730 mm", "4.9 GB"],
        ["index-5201", "2.8′ 〜 4′", "2730 〜 3900 mm", "9.3 GB"],
        ["index-5200", "2′ 〜 2.8′", "3900 〜 5450 mm", "15.9 GB"],
    ], st, widths=[42 * mm, 30 * mm, 46 * mm, CONTENT_W - 118 * mm],
        align=[None, "c", "c", "c"]))
    add(Paragraph("上の 3 段（4107〜4119 / 5206 / 5205）が基本セット "
                  "<b>AstrometryData.zip</b> に入っています。"
                  "それより長い焦点距離を使う人や、画面を切り出して短辺が狭い人は、"
                  "下の 5 つから必要なものを足してください。"
                  "迷ったら、<b>短辺の画角の前後 1 段ずつ</b>を入れておくと確実です"
                  "（デスクトップ版の「必要な段を選ぶ」はそうしています）。", st.body))
    ext(callout("★ 実際に踏んだ例 — 横幅だけ見ると足りない", [
        "焦点距離 818 mm・968 x 548 画素の画像（ごく普通の 16:9）で試したところ、"
        "横幅は 23.6′ なので index-4107（22′〜30′）で足りるはずでした。"
        "ところが<b>120 秒かけても解けませんでした</b>。",
        "短辺は <b>13.4′</b> です。<b>index-5205（11′〜16′）を足したら 10 秒で"
        "解けました。</b>切り出していない普通の画像でも、短辺で決まります。",
        "掩蔽観測でローリングシャッターの影響を抑えたり、コマ落ちを防ぐために"
        "読み出す範囲（特に<b>縦</b>）を切り詰めると、この差はさらに開きます。"
        "1936 x 400 画素に切り出すと、横 24′ に対して縦は <b>5.0′</b>。"
        "index-5202（4′〜5.6′）まで要ります。",
        "デスクトップ版の［準備］タブは、［解析］タブで画像を選んでおくと"
        "<b>その画像の縦横を読んで短辺のぶんまで選びます</b>。"
        "解けなかったときは、短辺の画角と必要なファイル名を画面に出します。",
    ], st))
    ext(callout("画角が合っていないときは、プログラムが教えてくれます", [
        "解析に失敗すると、手持ちの星図データが対応する画角と、"
        "足りない段のファイル名が画面に出ます。そのとおりに追加してください。",
        "2′ より狭い画角（ASI290MM で 9600 mm 超）は配布されていません。"
        "ビニングやレデューサーで画角を広げてください。",
    ], st, kind="note"))

    add(PageBreak())

    # ======================================================= 6. しくみ ===
    ext(h1("動作のしくみ", st, "7"))
    ext(steps([
        "<b>画像を読む</b> — FITS でも JPEG でも、カラーでも動画の 1 コマでも"
        "読めるようにしてあります。動画の場合は複数コマを重ねて暗い星を出します。",
        "<b>星を見つける</b> — 背景の明るさのむらを取り除き、"
        "ホットピクセルや、画面に焼き込まれた日時の文字を取り除いてから、"
        "本物の星だけを拾います。しきい値は星が十分見つかるまで自動で下げます。",
        "<b>星の並びを照合する</b> — 拾った星の位置だけを Astrometry.net に渡します。"
        "4 個の星が作る四角形の形を星図データと突き合わせ、"
        "一致する場所を空全体から探します。画像そのものは渡さないので、"
        "上下が入れ替わるような取り違えが起きません。",
        "<b>座標に直して描く</b> — 見つかった対応関係から、"
        "画像のどの点が空のどこかを計算し、目標への矢印を引きます。",
    ], st))
    add(Paragraph("インターネットにつながっているときは 3 の照合を"
                  "Astrometry.net のサーバーに任せます。つながっていないときは、"
                  "WSL の中の同じエンジンが、C:\\AstrometryData の星図データを"
                  "使って同じことをします。<b>どちらでも結果は同じ</b>です。", st.body))

    # =================================================== 7. トラブル ===
    ext(h1("うまくいかないとき", st, "8"))
    ext(table([
        ["症状", "考えられる原因と対処"],
        ["「画像が選ばれていません」と出る",
         "セル A のボタンでファイルを選んでいません。"
         "選び直すか、セル C の IMAGE_PATH にパスを直接書いてください。"],
        ["解析に失敗する（星が少ない）",
         "露出を伸ばす、ピントを合わせる、雲を確認する。"
         "画面に「星を◯個検出」と出るので、10 個を下回るようなら"
         "撮影条件を見直してください。"],
        ["解析に失敗する（画角が合わない）",
         "6 章の表を見て、自分の画角に合う星図データを追加してください。"
         "失敗時のメッセージに必要なファイル名が出ます。"],
        ["オフラインで「WSL が使えません」",
         "4.4 をやり直してください。管理者のターミナルでないと入りません。"],
        ["オフラインで「solve-field がありません」",
         "4.5 の apt install が終わっていません。"],
        ["オフラインで「星図データが見つかりません」",
         "4.6 の展開先か、4.7 の add_path が間違っています。"
         "C:\\AstrometryData の直下に index-....fits がある状態にしてください。"],
        ["オフライン解析がとても遅い",
         "4.7 の「速くしたい人へ」を実施してください。"
         "また FOCAL_LENGTH_MM を書くと探索範囲が決まり、大幅に速くなります。"],
        ["目標に矢印が向かない／位置がおかしい",
         "目標の名前や座標が間違っている可能性があります。"
         "画面に出る「画像中心」の座標が、撮ったつもりの場所と合っているか"
         "確かめてください。"],
        ["図の日本語が □□□ になる",
         "日本語フォントが見つかっていません。ふつうの Windows なら起きませんが、"
         "起きた場合はセル A の出力に「日本語フォントなし」と表示されます。"],
    ], st, widths=[52 * mm, CONTENT_W - 52 * mm]))

    add(PageBreak())

    # ========================================================== 付録 ===
    ext(h1("用語", st, "付録 A"))
    ext(table([
        ["用語", "意味"],
        ["赤経（RA）", "空の東西方向の座標。地球でいう経度にあたります。"],
        ["赤緯（Dec）", "空の南北方向の座標。地球でいう緯度にあたります。"],
        ["分角（′）", "角度の単位。1 度の 60 分の 1。満月の見かけの大きさが約 31′。"],
        ["秒角（″）", "1 分角の 60 分の 1。"],
        ["画角", "写真 1 枚に写る空の広さ。焦点距離が長いほど狭くなります。"],
        ["画素スケール", "1 画素が空の何秒角にあたるか。"],
        ["WCS", "画像の画素と空の座標の対応表。解析の答えそのものです。"],
        ["プレートソルブ", "写真から空の位置を割り出すこと。このプログラムの中心機能です。"],
        ["index ファイル", "星の並びのパターンを収めた星図データ。照合に使います。"],
        ["WSL", "Windows の中で Linux を動かすしくみ。オフライン解析に使います。"],
        ["FITS", "天文でよく使う画像形式。撮影条件などの情報も一緒に入っています。"],
    ], st, widths=[32 * mm, CONTENT_W - 32 * mm]))

    ext(h1("参考リンクと出典", st, "付録 B"))
    add(Paragraph("配布ページ（ノートブック・星図データ）", st.h3))
    ext(bullets([
        "https://cloud.akashi-kosaku.uk/s/M2sNsnA2K4oJ4Co",
        "https://github.com/Akashi3060/zahyou",
    ], st))
    add(Paragraph("星図データの原典", st.h3))
    ext(bullets([
        "5200 シリーズ（LITE）： "
        "https://portal.nersc.gov/project/cosmo/temp/dstn/index-5200/LITE/",
        "4100 シリーズ： https://data.astrometry.net/4100/",
    ], st))
    add(Paragraph("使用しているソフトウェア", st.h3))
    ext(bullets([
        "Astrometry.net（Lang et al. 2010, AJ 139, 1782）— 解析エンジン",
        "Astropy — 座標と FITS の取り扱い",
        "SciPy / NumPy / matplotlib / Pillow / reproject",
        "SIMBAD・VizieR（CDS, ストラスブール）— 天体名から座標を引くのに使用",
    ], st))
    add(HRule(CONTENT_W))
    add(Paragraph("このマニュアルの本文は Noto Sans JP（SIL Open Font License 1.1）"
                  "で組んでいます。", st.small))

    doc.multiBuild(f)
    return out_path


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        HERE, "..", "天体画像解析プログラム「zahyou」導入・運用マニュアルv3.pdf")
    p = build(os.path.abspath(out))
    print(f"wrote {p}  ({os.path.getsize(p)/1e6:.2f} MB)")
