"""
天体画像解析プログラム「zahyou」導入・運用マニュアル v4 を PDF で作る。
(v4 = zahyou v7(正式版 7.0.0)。デスクトップ版は Rust の exe、オフライン解析は zahyou の中のエンジン。
 2026-10-16 の決定(API キーの欄・DirectX 12 を既定)を入れ、2026-10-18 に β版の本文から正式版の本文に書き直した)

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
from manual_figures import ArrowLegend, FoldHint, NotebookLayout
from manual_style import (ACCENT, CONTENT_W, HRule, INK, JP, JPB, MARGIN,
                          MUTED, PAGE_H, PAGE_W, RULE, S, TocMark, bullets,
                          callout, code_block, h1, h2, steps, table)

HERE = os.path.dirname(os.path.abspath(__file__))
ASSETS = os.path.join(HERE, "assets")
VERSION = "v4"
DATE = "2026年10月"


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


SHOTS = os.path.join(os.path.dirname(HERE), "docs", "img")


def fig(name, width_mm, caption, st):
    path = os.path.join(ASSETS, name)
    if not os.path.exists(path):
        path = os.path.join(SHOTS, name)
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
                    title=f"天体画像解析プログラム「zahyou」導入・運用マニュアル {VERSION}",
                    author="zahyou", subject="導入・運用マニュアル",
                    # 同じ make_manual.py と同じフォントからは、いつ組んでも同じバイトの PDF にする
                    # (作成日時などを固定する。rs/tools/make_release_assets.py が組み直して突き合わせる)
                    invariant=1)
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
        ["v4（zahyou v7）で変わったこと", ""],
        ["オフライン解析", "zahyou の中のエンジンで解きます。"
         "<b>WSL も astrometry.net も要りません</b>（要るのは星図データだけ）"],
        ["解析の速さ", "オフラインで 1 枚 0.1 〜 0.4 秒"
         "（0.5 〜 4 メガ画素の実写。v6 の WSL 版は約 4 秒）"],
        ["exe", "約 13 MB・窓が出るまで 1 秒ほど（v6 は 102 MB・約 4 秒）"],
        ["対応する画角", "4′ 〜 2000′（焦点距離およそ 4800 mm まで。v6 と同じ）"],
        ["版", "v7.0.0。v6 の［使い方］タブの「更新を確認」→「更新する」で入れ替わります（9.1）"],
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

    ext(h2("特徴", st, "1.1"))
    ext(bullets([
        "<b>ハイブリッド解析</b> — インターネットにつながっていれば "
        "Astrometry.net のサーバーを使い（API キーは初めから入っています。2.3）、"
        "つながっていなければ自分の PC の中だけで解析します。切り替えは自動です。"
        "PC の中の解析エンジンは zahyou に入っているので、"
        "ほかのソフトを入れる必要はありません。",
        "<b>完全オフライン対応</b> — 電波の届かない山の上でも動きます。",
        "<b>導入量が数字で出る</b> — 「東へ 0.92 分角、北へ 1.3 分角」のように、"
        "赤道儀の軸に沿った移動量が直接わかります。",
        "<b>どんな画像でもそのまま</b> — カラーでも白黒でも、動画から切り出した"
        "1 コマでも、日時が焼き込まれていても構いません。焦点距離の設定も不要です。",
        "<b>星図も読めます</b> — 白地に黒い星の星図（Occult などが描くもの）は"
        "明暗が逆ですが、それを見分けて自動で反転してから解きます。"
        "撮った写真と星図を、同じやり方で突き合わせられます。",
    ], st))

    ext(h2("2 つの入り口 — どちらを使うか", st, "1.2"))
    add(Paragraph("<b>解き方はどちらも同じ</b>で、同じ画像なら同じ答えになることを"
                  "確かめてあります（7 章）。デスクトップ版は Rust、ノートブック版は Python で"
                  "書いてあり、デスクトップ版のほうが速く動きます（3.2）。"
                  "はじめての方や、とにかく動かしたい方は"
                  "<b>デスクトップ版</b>をどうぞ。", st.body))
    ext(table([
        ["", "デスクトップ版（exe）", "ノートブック版"],
        ["入れるもの", "<b>zahyou.exe 1 個だけ</b>", "Python + VS Code + ノートブック"],
        ["使い方", "ダブルクリック → 画像を選ぶ → 実行", "セルを上から実行"],
        ["オフラインの用意", "<b>［準備］タブのボタン 1 つ</b>",
         "4.4（星図データを置く）を自分で"],
        ["中身を読む・直す", "できません", "できます"],
        ["この本のどこ", "<b>2 章</b>", "4 章・5 章"],
    ], st, widths=[24 * mm, 58 * mm, CONTENT_W - 82 * mm]))

    ext(fig("app-original.png", 150, "図 1　撮影したときと同じ向きの画像。"
            "水色の × が画面の中心、黄色の ○ が目標、赤い矢印が動かす向き。", st))
    ext(fig("app-northup.png", 132, "図 2　北を上・東を左にそろえ直した画像。"
            "赤道儀の軸の向きと一致するので、こちらの方が操作しやすい人もいます。", st))

    add(PageBreak())

    # =================================================== 2. デスクトップ版 ===
    ext(h1("デスクトップ版（exe）を使う — こちらがおすすめ", st, "2"))
    add(Paragraph("<b>zahyou.exe を落として、ダブルクリックするだけです。</b>"
                  "Python も VS Code もインストールも要りません。"
                  "ファイルは 1 個（約 13 MB）だけです。", st.body))

    ext(h2("落として起動する", st, "2.1"))
    ext(steps([
        "GitHub の Releases（付録 B）から <b>zahyou.exe</b> をダウンロードします。"
        "置き場所はどこでも構いません（デスクトップでも、USB メモリでも）。",
        "<b>ダウンロードの途中で、Windows が何度か止めます。</b>"
        "通し方は次の 2.2 に 1 つずつ書いてあります。",
        "ダブルクリックします。1 秒ほどで窓が出ます。",
    ], st))

    ext(h2("出てくる警告と、その通し方", st, "2.2"))
    add(Paragraph("zahyou には<b>開発元の署名（コード署名証明書）が付いていません</b>。"
                  "証明書は毎年お金がかかるもので、個人が配るソフトには"
                  "付いていないことがよくあります。", st.body))
    add(Paragraph("Windows は「まだ広く出回っていないファイル」を見慣れないものとして"
                  "扱うので、<b>落としてから起動するまでに最大 4 回止められます</b>。"
                  "中身を見て危ないと判断されたわけではありません。"
                  "順に通し方を見ていきます。"
                  "<b>作った本人の PC では 1 つも出ません</b>ので、"
                  "配る側は気づきにくい種類の警告です。", st.body))

    add(Paragraph("① 「zahyou.exe で行う操作を選んでください。」→［保存］", st.h3))
    ext(fig("dl-1-save.png", 96,
            "図 3　いちばん右の［保存］を押します。", st))

    add(Paragraph("② 「一般的にダウンロードされていません」→［…］→［保存］", st.h3))
    add(Paragraph("ゴミ箱の右にある<b>［…］</b>（その他の操作）を押すと、"
                  "メニューに<b>［保存］</b>が出ます。", st.body))
    ext(fig("dl-2-keep.png", 88,
            "図 4　［…］を押してから、メニューの［保存］。", st))

    add(Paragraph("③ 「開く前に、信頼できることを確認してください」"
                  "→［削除］の▽→［保持する］", st.h3))
    add(Paragraph("<b>［保持する］は「削除」ボタンの中に隠れています。</b>"
                  "「削除」の右の<b>▽</b>を押すと出ます。"
                  "ここまで押して、はじめてファイルが残ります。", st.body))
    ext(fig("dl-3-trust.png", 80,
            "図 5　「削除」の▽から［保持する］。", st))
    ext(callout("zahyou.exe が見当たらないときは、ここで止まっています", [
        "①〜③ を押し切るまで、フォルダーには "
        "<b>「未確認 841468.crdownload」</b>のような名前の"
        "書きかけのファイルが残ったままで、<b>exe は出来上がりません</b>。",
        "その場合は、もう一度ダウンロードして ①〜③ を押してください。",
    ], st))

    add(Paragraph("④ ダウンロード完了 →「ファイルを開く」", st.h3))
    ext(fig("dl-4-open.png", 100,
            "図 6　ここまで来れば、あとは開くだけです。", st))

    add(Paragraph("⑤ 青い「Windows によって PC が保護されました」"
                  "→「詳細情報」→「実行」", st.h3))
    add(Paragraph("<b>最初は「実行」ボタンがありません。</b>"
                  "「詳細情報」を押すと現れます。"
                  "ここまでで起動します。次からは警告は出ません。", st.body))
    ext(fig("run-1-smartscreen.png", 120,
            "図 7　「詳細情報」→「実行」。この図は再現したものです。", st))

    add(Paragraph("灰色の「スマート アプリ コントロール」が出たとき", st.h3))
    add(Paragraph("<b>これだけは「実行」で通せません。</b>"
                  "ボタンが［OK］と［Microsoft Store からアプリを入手してください］"
                  "しかなく、その場で許可する方法が用意されていません。"
                  "署名の無いアプリを一律に止める仕組みだからです。"
                  "Windows 11 を新しく入れた PC で有効になっていることがあります。",
                  st.body))
    ext(fig("run-2-smart-app-control.png", 120,
            "図 8　通すボタンがありません。この図は再現したものです。", st))
    add(Paragraph("次の<b>どちらか</b>を選んでください。", st.body))
    ext(table([
        ["", "やり方", "引きかえに"],
        ["A. ノートブック版を使う",
         "4 章へ進みます。exe を使わないので、"
         "スマート アプリ コントロールは関係ありません",
         "Python と VS Code を入れる手間"],
        ["B. 機能をオフにする",
         "Windows セキュリティ →「アプリ &amp; ブラウザー コントロール」"
         "→「スマート アプリ コントロールの設定」→ <b>オフ</b>",
         "PC 全体の保護が 1 つ減ります"],
    ], st, widths=[34 * mm, CONTENT_W - 72 * mm, 38 * mm]))
    ext(callout("オフにする前に知っておくこと", [
        "スマート アプリ コントロールは、<b>一度オフにすると戻しにくい</b>"
        "作りになっています。もともとは Windows を入れ直さないと"
        "戻せませんでしたが、最近の更新プログラムが入っていれば"
        "入れ直さずに戻せることもあります。",
        "<b>署名の無いソフトを一律に止める</b>仕組みなので、オフにすると"
        " zahyou 以外にも効かなくなります。ここは PC を使う本人の判断です。"
        "決めかねるときは、A のノートブック版を選んでください。",
    ], st))

    add(Paragraph("そのほか", st.h3))
    ext(bullets([
        "<b>ウイルス対策ソフトが誤検知することがあります</b>"
        "（署名の無い、まだ広く出回っていない exe なので）。"
        "その場合は、ウイルス対策ソフト側で zahyou.exe を除外に登録してください。",
        "<b>中身が本物か確かめたい方は</b>、GitHub の Releases（付録 B）に付けている "
        "SHA256SUMS.txt の SHA256 と、手元のファイルのものを見比べてください。"
        "<b>確かめなくても使えます。</b>気になる方向けの補足です。"
        "PowerShell で次を打つと出ます。",
    ], st))
    ext(code_block([
        "Get-FileHash .\\zahyou.exe -Algorithm SHA256",
    ], st))

    add(Paragraph("新しい版が出たときの入れ替え方と、やめるときの消し方は "
                  "<b>9 章</b>にあります。", st.small))

    ext(h2("使い方 — 4 つだけ", st, "2.3"))
    ext(steps([
        "<b>［解析］</b>タブの「参照...」を押して、天体画像を選びます"
        "（窓へドラッグしても選べます）。",
        "目標を指定します。<b>天体名</b>（UCAC4 660-021020 など）か、"
        "<b>赤経・赤緯</b>のどちらかです。",
        "<b>「解析を実行」</b>を押します。",
        "画面の上に出る<b>3 つの数字</b>が答えです。"
        "赤経・赤緯それぞれに何分角動かせばよいかを表しています。",
    ], st))
    ext(fig("app-analyze.png", 150,
            "図 9　解析が終わったところ。ふだん触るのは左の列と"
            "「解析を実行」だけです。", st))
    add(Paragraph("インターネットがあれば、ここまでで動きます（オンライン解析の API キーは"
                  "初めから入っています。この節の終わり）。ネットの無いところでは、"
                  "星図データ（2.4）を落としておけば同じように動きます。"
                  "設定（目標・焦点距離・配色など）は次に開いたときも残ります。",
                  st.body))

    add(Paragraph("出た数字の読み方", st.h3))
    ext(fig("app-numbers.png", 150,
            "図 10　この 3 つが答え。下の細い行は画像中心の座標・"
            "画素スケール・視野・かかった時間。", st))
    add(Paragraph("いま向いている点（画像の中心）から目標まで、"
                  "<b>赤経方向に 0.916′ 東へ、赤緯方向に 1.305′ 北へ</b>"
                  "動かせばよい、という意味です。"
                  "左端の「距離」は、その 2 つを合わせた実際のずれです。", st.body))
    add(Paragraph("図 9・図 10 はネットにつながっていない状態で撮ったもので、「所要 0.1 秒」は"
                  "オフラインで解いた時間です。ネットにつながっていると、天体名を調べ直したり、"
                  "先にオンライン解析を試したりするぶん延びます。", st.small))

    add(Paragraph("図は 2 枚出ます", st.h3))
    add(Paragraph("<b>「撮影時の向き」</b>は撮ったままの向き、"
                  "<b>「北が上」</b>は北を上・東を左にそろえたものです。"
                  "赤道儀の軸操作と向きが揃うので、こちらが読みやすい人もいます。"
                  "<b>はじめて使うときは「撮影時の向き」</b>が開き、"
                  "切り替えると<b>次からはその表示で出ます</b>。", st.body))
    ext(fig("app-northup.png", 118,
            "図 11　「北が上」に切り替えたところ。水色の × が画像の中心、"
            "黄色の ○ が目標、赤い矢印が動かす向き。", st))
    add(Paragraph("図の下のボタンで、拡大して星の位置を確かめたり、"
                  "PNG で保存したりできます。", st.body))
    ext(fig("app-toolbar.png", 100,
            "図 12　図の下のボタン。", st))
    add(Paragraph("図はマウスのホイールで拡大・縮小、ドラッグで移動できます。"
                  "「全体」で元に戻ります。", st.small))

    add(Paragraph("ネットにつながっているとき（オンライン解析）", st.h3))
    add(Paragraph("左の列の「方法」が<b>「自動」</b>（既定）なら、ネットがあれば先に "
                  "nova.astrometry.net に問い合わせ、解けなければ zahyou の中で解きます。"
                  "問い合わせに使う <b>API キー</b>は、左の列の「打ち切り」の下にある"
                  "<b>「API キー」の欄</b>に入っています（窓が小さいときは、左の列を下へ送ると出ます）。"
                  "<b>はじめは作者のキー（既定）が入っているので、そのままで使えます。</b>"
                  "自分のキー（nova.astrometry.net に登録すると、自分のページで見られます）を"
                  "使うなら、欄に貼り付けてください。", st.body))
    ext(fig("app-apikey.png", 70,
            "図 12-2　API キーの欄。中身は伏せ字で、「表示」で見え、"
            "「元に戻す」で既定のキーに戻ります。", st))
    ext(bullets([
        "キーは設定（%LOCALAPPDATA%\\zahyou\\gui.json）に覚えます。",
        "<b>欄を空にすると、ネットがあっても nova.astrometry.net には問い合わせず、"
        "はじめからオフラインで解きます</b>（「オンラインのみ」は欄が空だと始められません）。",
    ], st))
    # 2026-10-16 の決定: 画像は同意を取らずに送ってよい。その代わり、何が送られるかと止め方を書く(README・［使い方］タブと同じ)
    add(Paragraph("nova.astrometry.net へ送るもの", st.h3))
    add(Paragraph("方法が「自動」か「オンラインのみ」で、API キーの欄にキーがあり、ネットにつながっていると、"
                  "<b>送る前に確かめることはせずに</b>次のものを送ります。", st.body))
    ext(steps([
        "まず、<b>検出した星の位置</b>（画像の中の座標の一覧）と画像の大きさ。"
        "目標の方向や画素スケールが分かっていれば、探す範囲の手がかりも送ります。",
        "星の位置だけで解けなければ、<b>画像ファイルそのもの</b>（ファイル名も）。"
        "星を 1 つも検出できなかったときは、1 を飛ばしてはじめから画像ファイルを送ります。",
    ], st))
    # 2026-10-19 に分かった: v6 の exe は画像ファイルを送っていなかった(同梱の astroquery 0.4.10 は photutils があると
    # 画像を上げずに自分で星を数えて位置を送る)。「v6 と同じです」と書いていたのを直した(README と同じ)
    add(Paragraph("送る前に聞かないのは v6 と同じですが、<b>exe が画像ファイルを送るのは v7 からです</b>。"
                  "v6 の exe は、2 で画像ファイルを送らずに、画像から星を数え直した位置をもう一度送っていました。"
                  "中で使っていた astroquery が、同梱の photutils があるとそう動くためで、"
                  "FITS 以外の画像ではここでオンライン解析が失敗していました。"
                  "画像ファイルまで送っていたのは、photutils を入れずに動かした v6 のノートブック版です。", st.body))
    # 2026-10-16 の 3 回目の決定: 公開しない指定(publicly_visible = n)を付けて送る。それでも第三者に見られうると書く
    add(Paragraph("v7 は 1 も 2 も「公開しない」の指定（publicly_visible = n）を付けて送ります。"
                  "<b>ただし、nova.astrometry.net に送ったものは第三者に見られることがあります</b>。"
                  "2026-10-18 に実際に送って、ログインしていない第三者の立場で確かめると、"
                  "nova のトップ・画像の一覧と詳細ページには出ませんでしたが、"
                  "検索ページ・利用者のページからは送った時刻・件数と API キーの持ち主のアカウント名が、"
                  "番号が分かれば API などから解いた位置と向き・送った星の座標・星を点で描いた絵が見えました。"
                  "星の位置だけで解けたときは、写真そのものは送りません。", st.body))
    ext(callout("送りたくないときは", [
        "「方法」を<b>「オフラインのみ」</b>にするか、<b>API キーの欄を空</b>にしてください。",
        "どちらでも、解析は zahyou の中だけで行い、星の位置も画像も nova.astrometry.net へは"
        "送りません（星図データが要ります。2.4）。",
    ], st, kind="note"))

    ext(h2("ネットの無いところで使う — ［準備］タブ", st, "2.4"))
    add(Paragraph("<b>「まとめて準備する」を押すだけです。</b>"
                  "足りない星図データ（おすすめの段）を落として、"
                  "1 本ずつ中身（途中で切れていないか）を検査します。"
                  "途中で止めても、次に押せば続きから落とし直します。", st.body))
    add(Paragraph("解析のエンジンは zahyou の中に入っているので、"
                  "<b>要るのは星図データだけ</b>です"
                  "（v6 まではここで WSL と astrometry.net も入れていました）。"
                  "一覧表は「星図データ」「検査」「総合」の 3 行で、"
                  "「総合」が<b>「オフライン解析できます」</b>になれば完了です。"
                  "「状態を確認」を押すと、置き場所のファイルを数え直して検査し直します。",
                  st.body))
    ext(fig("app-prepare.png", 150,
            "図 13　用意できたところ。3 つとも OK になれば、"
            "ネットの無い場所でも解けます。", st))

    # 見出しだけがページの下に残らないよう、図と一緒に置く
    add(KeepTogether([Paragraph("星図データの選び方 — ボタンは 2 つ", st.h3)]
                     + fig("app-index.png", 82,
                           "図 14　星図データの枠。上のボタンが「おすすめ」、"
                           "下が焦点距離から絞るほう。持っている段には「入っています」と出ます。", st)))
    ext(table([
        ["ボタン", "入るもの", "こんなとき"],
        ["<b>おすすめをまとめて落とす</b>",
         "画角 <b>4′ 以上</b>をぜんぶ<br/>（18 段・約 9.5 GB）",
         "<b>機材の数値が分からない</b><br/>数字を何も入れずに押せます"],
        ["<b>必要な段を選ぶ</b>",
         "その画角に要る段だけ<br/>（数百 MB で済むことも）",
         "焦点距離とセンサー横幅が分かる<br/>容量を節約したい"],
    ], st, widths=[44 * mm, 46 * mm, CONTENT_W - 90 * mm]))
    add(Paragraph("どちらでも、<b>すでに持っている段は落とし直しません</b>。"
                  "どの段が要るかは 6.2 の早見表でも確かめられます。", st.small))
    ext(callout("「おすすめ」で足りないのは、画角 4′ より狭いときだけです", [
        "<b>ふつうの望遠鏡はこれで足ります。</b>1/1.8 型のカメラなら、"
        "焦点距離 4000 mm までが 4′ 以上に収まります。",
        "外してあるのは <b>index-5200 と 5201（画角 2′〜4′）の 2 段だけ</b>で、"
        "これが合わせて <b>24.6 GB</b> あります。ここを入れないので 9.5 GB で"
        "済んでいます。掩蔽観測で画面を大きく切り出す方は、"
        "6.2 を見て、この 2 段を手で足してください。",
    ], st, kind="note"))
    ext(callout("画面を切り出している人は、画像を選んでから押してください", [
        "「必要な段を選ぶ」のほうは、［解析］タブで画像を選んでおくと"
        "<b>その画像の縦横を読んで</b>短辺のぶんまで含めて段を選びます。"
        "掩蔽観測で縦を切り詰めている場合、これをしないと足りません"
        "（理由は 6.1）。",
        "「おすすめ」のほうは画像を見ません。4′ 以上をまとめて入れるので、"
        "切り出していても足ります。",
    ], st))
    ext(h2("暗いところで使う", st, "2.5"))
    add(Paragraph("<b>右下の「暗く」</b>（月の印が付いたボタン）を押すと、"
                  "画面もタイトルバーも図の中も黒基調になります。"
                  "観測中に目が眩みません。もう一度押すと戻ります。", st.body))
    add(Paragraph("この配色は次に開いたときも残ります。"
                  "現地に着いてから押し直す必要はありません。", st.small))
    ext(fig("app-dark.png", 150,
            "図 15　暗い配色にしたところ。図の中まで黒基調になります。", st))

    ext(h2("目標が分からないとき — 完全ブラインド", st, "2.6"))
    add(Paragraph("目標に<b>「指定しない（完全ブラインド）」</b>を選ぶと、"
                  "目標を決めずに<b>「この画像はどこを向いているか」だけ</b>を"
                  "求めます。矢印は出ません。", st.body))
    ext(fig("app-target.png", 62,
            "図 16　目標の枠。いちばん下がブラインド。", st))
    add(Paragraph("上の 3 つの数字は、<b>画像中心の赤経・赤緯・画素スケール</b>に"
                  "変わります。目標が画面の外にあるのか分からないときは、"
                  "まずこれで向きを確かめてください。", st.body))
    ext(fig("app-blind-numbers.png", 150,
            "図 17　ブラインドのときの 3 つ。目標を指定したときとは"
            "中身が変わります。", st))
    add(Paragraph("画素スケールが出れば、そこから焦点距離を逆算できます"
                  "（式は 6.1）。機材の数値が分からないときにも使えます。",
                  st.small))

    ext(callout("exe を使う方は、ここで 2 章はおしまいです", [
        "<b>3 章</b>は必要なパソコンの目安、"
        "<b>4 章・5 章はノートブック版の説明</b>です。"
        "exe だけを使う方は読む必要がありません。",
        "次に読むとよいのは <b>6 章（画角と星図データ）</b>です。"
        "オフライン解析でどの段を入れればよいかが分かります。"
        "うまく動かないときは <b>8 章</b>、"
        "新しい版への入れ替えとアンインストールは <b>9 章</b>にあります。",
    ], st, kind="note"))

    add(PageBreak())

    # =========================================================== 3. 準備 ===
    ext(h1("準備するもの", st, "3"))
    add(Paragraph("<b>想定しているのは、Windows 11 が動く、RAM 8 GB 以上のノート PC</b> です。"
                  "デスクトップ版（exe）に要るものは次のとおりです。"
                  "ノートブック版は、これに Python と VS Code が加わります（4 章）。", st.body))
    ext(table([
        ["", "要るもの"],
        ["OS", "Windows 11（<b>x64</b>）。Windows 10 では確かめていません"],
        ["メモリ", "8 GB 以上（推奨 16 GB）。zahyou だけが使う分は 24 メガ画素の画像の表示中で"
         " 0.5 GB ほどなので、8 GB でも足りる見込みです（実測は 3.1）"],
        ["ディスク", "zahyou.exe 約 13 MB ＋ 星図データ"
         "（基本セット 1.2 GB、4′ まで入れるなら 9.1 GB。3.3）"],
        ["画面の描画", "<b>DirectX 12</b>（既定）。主の画面をつないでいる GPU で描きます"
         "（ノート PC ではふつう内蔵 GPU）。開けなければ OpenGL、それでも開けなければ"
         "ソフトウェア描画（WARP）に自動で切り替えます"],
        ["画面の大きさ", "窓は 1000 x 660 まで縮められます"],
        ["ネット", "オフライン解析だけなら、星図データを落とすときだけ"],
    ], st, widths=[30 * mm, CONTENT_W - 30 * mm]))
    ext(callout("確かめた PC は 2 台です", [
        "<b>ノート PC（Core i7-1360P・RAM 16 GB・SSD 1 TB）</b>で、軽快に動くことを確かめました。",
        "この版の数字は、すべて <b>Core Ultra 7 265K（20 スレッド）・Windows 11</b> の"
        "デスクトップ PC で測ったものです（GPU は NVIDIA GeForce RTX 3060 と内蔵の Intel Graphics の"
        "両方で描いて確かめました）。描き方の切り替え（DirectX 12 → OpenGL → ソフトウェア描画）は、"
        "わざと開けなくして確かめてあります。",
        "<b>RAM 8 GB の PC・リモート デスクトップ・GPU のドライバーが無い PC では、"
        "まだ確かめていません。</b>メモリの「推奨 16 GB」は、確かめたノート PC に合わせたものです。",
        "どの描き方でも開けないときは、理由を箱で知らせます（8 章）。",
    ], st, kind="note"))
    add(Paragraph("描き方を手で選ぶこともできます。zahyou.exe の後ろに次を付けて起動します"
                  "（ショートカットのリンク先に足すなど）。", st.body))
    ext(table([
        ["付けるもの", "描き方"],
        ["--glow", "OpenGL で描く（開けなくても、ほかの描き方を試さない）"],
        ["--dx12", "DirectX 12 で描く（同上）"],
        ["--gpu low / --gpu high", "DirectX 12 で、内蔵 GPU / 外付け GPU を先に使う"],
        ["--gpu warp", "DirectX 12 のソフトウェア描画（GPU を使わない。遅い）"],
    ], st, widths=[46 * mm, CONTENT_W - 46 * mm], align=["id", None]))

    ext(callout("ARM 版の Windows 向けには建てていません", [
        "zahyou.exe は <b>x64</b> の exe です。"
        "Snapdragon X などの ARM 機での動き方は確かめていません。",
    ], st))

    # 見出しだけがページの下に残らないよう、表までひとかたまりにする
    add(KeepTogether(h2("メモリと描画（実測）", st, "3.1") + [
        Paragraph("zahyou だけが使う分（私有）と working set（合計）です。合成の FITS を開いて"
                  "図を拡大・移動させる物差しで、3 回の中央値を取りました"
                  "（GPU は NVIDIA GeForce RTX 3060）。", st.small)] + table([
        ["", "DirectX 12（既定）", "OpenGL"],
        ["画面を出しただけ（起動から 3 秒）", "248 / 175 MB", "131 / 128 MB"],
        ["24 メガ画素（6000 x 4000）の図を表示中", "529 / 324 MB", "457 / 310 MB"],
        ["61 メガ画素（9504 x 6336）の図を表示中", "906 / 549 MB", "826 / 493 MB"],
    ], st, widths=[76 * mm, 46 * mm, CONTENT_W - 122 * mm], align=[None, "c", "c"])))
    ext(bullets([
        "61 メガ画素の画像を読み込む瞬間の私有の最大は 1,289 〜 1,458 MB でした（DirectX 12）。",
        "内蔵 GPU（Intel Graphics）で描くと、描く絵が RAM に載るので working set が大きくなります"
        "（61 メガ画素の表示中で 948 MB）。",
        "図の拡大・移動のフレームの間隔は、中央値で画面の書き換え 1 回分（75 Hz で 13.3 ミリ秒）でした"
        "（24・61 メガ画素、DirectX 12・OpenGL・内蔵 GPU とも）。",
        "解析そのもののメモリ（Python のエンジンとの比べ）は docs/rust-port.md の 7.3 節にあります"
        "（描き方を今の形にする前の値です）。",
    ], st))

    ext(h2("解析にかかる時間（実測）", st, "3.2"))
    add(Paragraph("画像の読み込みから図までの時間です。焦点距離も手がかりも入れず、"
                  "星図データは 4′〜2000′ の全部（253 ファイル）を置いた状態で測りました。"
                  "Python の図の時間は、図を作るまで（画面に描く・PNG にするのは含みません）。",
                  st.small))
    ext(table([
        ["画像", "exe", "Python"],
        ["968 x 548 の動画の 1 コマ・目標あり", "0.12 秒", "2.13 秒"],
        ["同・目標なし", "0.09 秒", "0.69 秒"],
        ["1608 x 1104 の 16 bit FITS・目標なし", "0.11 秒", "1.54 秒"],
        ["2015 x 1001 の星図（PNG）・目標なし", "0.31 秒", "5.75 秒"],
        ["2704 x 1536 の彗星の FITS・目標なし", "0.36 秒", "6.09 秒"],
        ["24 メガ画素（6000 x 4000 の合成）・目標あり", "1.12 秒", "37.1 秒"],
        ["同・目標なし", "1.07 秒", "20.5 秒"],
        ["焦点距離の書き違い（2500 mm と入力、実際は 818 mm）",
         "26.3 秒", "132.4 秒"],
    ], st, widths=[92 * mm, 28 * mm, CONTENT_W - 120 * mm], align=[None, "c", "c"]))
    add(Paragraph("起動は、最初の描画まで <b>0.37 〜 0.40 秒</b>（DirectX 12、3 回）・"
                  "0.20 〜 0.21 秒（OpenGL、3 回）でした（v6 は窓が出るまで約 4 秒）。"
                  "解析の時間はβ版で測り直しても差がありませんでした。"
                  "解けない画像は、解析の制限時間（既定 300 秒）まで探して終わります"
                  "（過ぎるのは 0.006 秒以内でした）。"
                  "解析中に「中止」を押すと 0.03 秒で止まりました。", st.body))
    add(Paragraph("CPU のコア数を変えての測定はしていません。", st.small))

    add(KeepTogether(h2("ディスクの内訳", st, "3.3") + table([
        ["何に使うか", "大きさ", "いつ要るか"],
        ["zahyou.exe", "約 13 MB", "いつも"],
        ["設定と記憶", "数 KB", "いつも"],
        ["星図データ 基本セット（11′〜2000′）", "1.2 GB", "オフライン解析のとき"],
        ["＋ 4′ まで（5204 / 5203 / 5202）", "計 9.1 GB", "長焦点・切り出しのとき"],
        ["＋ 2′ まで（5201 / 5200）", "計 34 GB", "さらに狭いとき"],
    ], st, widths=[66 * mm, 32 * mm, CONTENT_W - 98 * mm],
        align=[None, "c", None])))
    add(Paragraph("<b>狭い画角ほど重くなります。</b>9.1 GB のうち 7.0 GB は "
                  "index-5202 と 5203（4′〜8′）です。"
                  "4′ まで要らなければ、容量は一桁下がります。", st.body))
    add(Paragraph("v6 の exe は起動のたびに 232 MB を一時展開していましたが、"
                  "v7 の exe は展開せずにそのまま動きます。"
                  "デスクトップ版の［準備］タブから入れる場合は、"
                  "必要なファイルを直接落とすので<b>zip の置き場所は要りません</b>。"
                  "配布ページの zip から入れるときは、"
                  "展開のあいだ一時的に 2 倍の空きが要ります。", st.small))

    ext(callout("ノートブック版をオンラインだけで使うなら、4.4 は飛ばせます", [
        "インターネットにつながる場所でしか使わないのであれば、"
        "星図データ（1.2 GB ほど。細かい段まで入れるなら 9 GB ほど）は要りません。"
        "4.1 〜 4.3 と 4.5 だけ済ませてください。",
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
        "<b>zahyou_v7.ipynb</b> と <b>zahyou_engine.py</b> の 2 つ"
        "（GitHub の Releases か配布ページにあります。付録 B）を、"
        "分かりやすい場所（デスクトップに作った zahyou フォルダーなど）に"
        "<b>そろえて</b>置きます。片方だけでは動きません。",
        "<b>zahyou_v7.ipynb</b> を右クリック →「プログラムから開く」→ VS Code を選びます。",
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

    ext(h2("【オフラインで使う人だけ】星図データを置く", st, "4.4"))
    add(Paragraph("オフライン解析のエンジンも zahyou_engine.py に入っています。"
                  "<b>WSL も astrometry.net も入れる必要はありません</b>"
                  "（v3 の 4.4 〜 4.7 の手順は要らなくなりました）。"
                  "要るのは「どの星がどこにあるか」を書いた星図データ（index ファイル）だけです。"
                  "<b>使う望遠鏡の画角によって必要なものが変わります</b>ので、"
                  "6.2 の早見表で確かめてから落としてください。", st.body))
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
        "それより長い焦点距離を使う場合は、6.2 の表を見て "
        "<b>index-5204 / 5203 / 5202</b> を必要な分だけ足してください。",
    ], st, kind="note"))
    add(Paragraph("別の場所（外付けのドライブなど）に置いたときは、"
                  "セル C の <b>INDEX_DIR</b> にその場所を書きます。", st.body))
    ext(code_block(["INDEX_DIR = r'D:\\AstrometryData'"], st))
    ext(callout("￥ と \\ は同じ文字です", [
        "日本語環境の Windows では、フォルダーの区切りが <b>￥</b>（円マーク）で"
        "表示されることがあります。英語環境の <b>\\</b>（バックスラッシュ）と"
        "まったく同じ文字で、画面上の見た目が違うだけです。",
        "このマニュアルでは <b>\\</b> と書いていますが、お使いの PC で <b>￥</b> と"
        "出ていてもそのままで正しく、書き直す必要はありません。"
        "キーボードでは、どちらも右下の <b>￥</b> キーで打てます。",
    ], st, kind="note"))
    add(Paragraph("デスクトップ版の［準備］タブで落とした星図データも同じ形で置かれるので、"
                  "同じフォルダーをそのまま使えます。", st.small))

    ext(h2("動作確認", st, "4.5"))
    add(Paragraph("VS Code で zahyou_v7.ipynb を開き、セル A → セル B → セル C の順に"
                  "実行します。手元のテスト画像で結果の図が 2 枚出れば導入完了です。",
                  st.body))
    add(Paragraph("オフライン解析だけを試したいときは、セル C の "
                  "<b>SOLVE_MODE</b> を <b>'OFFLINE'</b> にして実行してください。",
                  st.body))
    ext(callout("どのくらい時間がかかるか（実測）", [
        "星図データ 253 ファイル（4′〜2000′）を "
        r"C:\AstrometryData に置いた構成で、"
        "968 × 548 画素の画像を、目標ありで <b>2.1 秒</b>、目標なしで <b>0.7 秒</b>で"
        "解けました（画像の読み込みから図を作るまで。Core Ultra 7 265K）。",
        "同じ画像をデスクトップ版で解くと 0.1 秒ほどです（3.2）。",
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
         "'OFFLINE'（オフラインのみ）", "'AUTO'"],
        ["ONLINE_TIMEOUT", "ネット解析を諦めるまでの秒数", "120"],
        ["OFFLINE_TIMEOUT", "オフライン解析にかけてよい合計秒数", "300"],
        ["INDEX_DIR", "星図データの置き場所。既定は C:\\AstrometryData（4.4）", "—"],
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
        "    全角距離      : 1.595'  (95.7\")",
        "    赤経(RA) 方向 : 0.916' 東へ",
        "    赤緯(Dec)方向 : 1.305' 北へ",
    ], st))
    add(Paragraph("この例なら、赤経を東へ 0.92 分角、赤緯を北へ 1.3 分角動かせば"
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

    ext(h2("要る段は「短いほうの辺」で決まる", st, "6.1"))
    add(Paragraph("プログラムは星が作る四角形の形を星図と突き合わせますが、"
                  "その四角形は<b>短いほうの辺に収まる大きさまでしか作れません</b>。"
                  "そのため、要る段は横幅ではなく<b>短辺</b>で決まります。"
                  "<b>切り出していない普通の画像でも同じです。</b>", st.body))
    add(Paragraph("画角は次の式で求まります。", st.body))
    ext(code_block([
        "画素スケール[\"/px] = 206.265 x 画素サイズ[um] / 焦点距離[mm]",
        "画角[']            = 画像の横幅[px] x 画素スケール[\"/px] / 60",
        "短辺の画角[']       = 画像の高さ[px] x 画素スケール[\"/px] / 60",
    ], st))
    ext(callout("★ 実際に踏んだ例（v6 で）— 横幅だけ見ると足りない", [
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
        "<b>その画像の縦横を読んで短辺のぶんまで選びます</b>。",
    ], st))

    ext(h2("焦点距離の早見表", st, "6.2"))
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
    ], st, widths=[40 * mm, 28 * mm, 54 * mm, CONTENT_W - 122 * mm],
        align=[None, "c", "c", "c"]))
    add(Paragraph("上の 3 段（4107〜4119 / 5206 / 5205）が基本セット "
                  "<b>AstrometryData.zip</b> に入っています。"
                  "それより長い焦点距離を使う人や、画面を切り出して短辺が狭い人は、"
                  "下の 5 つから必要なものを足してください。"
                  "迷ったら、<b>短辺の画角の前後 1 段ずつ</b>を入れておくと確実です"
                  "（デスクトップ版の「必要な段を選ぶ」はそうしています）。", st.body))
    ext(callout("表を読まずに済ませたいなら —— 下の 2 段以外をぜんぶ", [
        "デスクトップ版の <b>「おすすめをまとめて落とす」</b>は、"
        "この表の <b>index-5202 までをぜんぶ</b>入れます（18 段・約 9.5 GB）。"
        "焦点距離もセンサー横幅も入れずに押せます。",
        "入らないのは<b>いちばん下の 2 段（5201 / 5200）だけ</b>で、"
        "この 2 つで <b>24.6 GB</b> を占めます。"
        "<b>ASI290MM で焦点距離 2730 mm を超える</b>ときや、"
        "縦を大きく切り詰めて短辺が 4′ を切るときは、"
        "この 2 段を［準備］タブのチェックで手を付けて足してください。",
    ], st, kind="note"))

    ext(h2("それでも解けないときは", st, "6.3"))
    ext(callout("画角が合っていないときは、プログラムが教えてくれます", [
        "解析に失敗すると、手持ちの星図データが対応する画角と、"
        "足りない段のファイル名が画面に出ます。そのとおりに追加してください。",
        "2′ より狭い画角（ASI290MM で 9600 mm 超）は配布されていません。"
        "ビニングやレデューサーで画角を広げてください。",
    ], st, kind="note"))
    add(Paragraph("星の数が足りない・写りが悪いなど、画角以外の原因は "
                  "<b>8 章</b>にまとめてあります。", st.body))

    add(PageBreak())

    # ======================================================= 6. しくみ ===
    ext(h1("動作のしくみ", st, "7"))
    ext(steps([
        "<b>画像を読む</b> — FITS でも JPEG でも、カラーでも動画の 1 コマでも"
        "読めるようにしてあります。動画の場合は複数コマを重ねて暗い星を出します。"
        "白地に黒い星の星図は、明るさの分布から見分けて自動で反転します"
        "（背景がいちばん明るく、そこから暗いほうへだけ大きく伸びていれば星図）。",
        "<b>星を見つける</b> — 背景の明るさのむらを取り除き、"
        "ホットピクセルや、画面に焼き込まれた日時の文字を取り除いてから、"
        "本物の星だけを拾います。しきい値は星が十分見つかるまで自動で下げます。",
        "<b>星の並びを照合する</b> — 拾った星の位置から、"
        "4 個の星が作る四角形の形を星図データと突き合わせ、"
        "一致する場所を空全体から探します（Astrometry.net と同じ方式。"
        "Lang et al. 2010 の論文をもとに、zahyou 用に書き直したエンジンが行います）。"
        "このエンジンは星の位置だけで照合し、画像を別の形式に作り変えて渡すことはしないので、"
        "上下が入れ替わるような取り違え（v5 で起きていた不具合）が起きません。",
        "<b>座標に直して描く</b> — 見つかった対応関係から、"
        "画像のどの点が空のどこかを計算し、目標への矢印を引きます。",
    ], st))
    add(Paragraph("インターネットにつながっていて API キーがあるときは、3 の照合を"
                  "Astrometry.net のサーバーに任せます。そのときもまず拾った星の位置だけを送り、"
                  "それで解けなければ画像ファイルそのもの（作り変えずに、選んだファイルのまま）を"
                  "送ります（2.3）。それ以外は、zahyou の中のエンジンが "
                  "C:\\AstrometryData の星図データを使って同じことをします。"
                  "v6 までは、この照合を WSL の中の astrometry.net に頼んでいました。"
                  "同じ方式なので答えはほぼ同じで、基準の実写では v6 の解と"
                  "画像中心が 0.9″ 違う程度でした。", st.body))
    add(Paragraph("デスクトップ版（Rust）とノートブック版（Python）は別々に書いたプログラムですが、"
                  "20 の場面で同じ画像を解かせて、解けたか・画像中心・画素スケール・"
                  "目標までのずれが同じになることを確かめています。", st.small))

    add(PageBreak())

    # =================================================== 7. トラブル ===
    ext(h1("うまくいかないとき", st, "8"))
    ext(table([
        ["症状", "考えられる原因と対処"],
        ["落としたのに zahyou.exe が無い",
         "ブラウザの警告に［保存］で答えていません。"
         "「未確認 ....crdownload」という書きかけのファイルが残っている"
         "はずです。<b>2.2 ①</b>を見てください。"],
        ["「スマート アプリ コントロールが……ブロックしました」",
         "灰色の画面で、この場では通せません。"
         "ノートブック版（4 章）を使うか、機能をオフにします。"
         "<b>2.2 ③</b>に、どちらを選ぶかの表があります。"],
        ["「Windows によって PC が保護されました」（青）",
         "<b>「詳細情報」→「実行」</b>で通せます。"
         "署名を付けていないためで、故障ではありません（<b>2.2 ②</b>）。"],
        ["「画像が選ばれていません」と出る",
         "セル A のボタンでファイルを選んでいません。"
         "選び直すか、セル C の IMAGE_PATH にパスを直接書いてください。"],
        ["解析に失敗する（星が少ない）",
         "露出を伸ばす、ピントを合わせる、雲を確認する。"
         "画面に「星を◯個検出」と出るので、10 個を下回るようなら"
         "撮影条件を見直してください。"],
        ["解析に失敗する（画角が合わない）",
         "6.2 の表を見て、自分の画角に合う星図データを追加してください。"
         "失敗時のメッセージに必要なファイル名が出ます。"],
        ["オフラインで「星図データが見つかりません」",
         "4.4 の展開先か、セル C の INDEX_DIR（exe は［準備］タブの置き場所）が"
         "違っています。C:\\AstrometryData の直下に index-....fits がある状態にしてください。"],
        ["「星図データを今は開けないので、この探索では使いません」（exe）",
         "ほかのプログラムがその星図データを書き込み用に開いています"
         "（コピーや展開の途中など）。終わるのを待ってから解き直してください。"],
        ["オフライン解析がとても遅い",
         "焦点距離（exe は左の列、ノートブック版は FOCAL_LENGTH_MM）を書くと"
         "探索範囲が決まり、大幅に速くなります。"],
        ["「API キーが空です」と出て「オンラインのみ」が始まらない（exe）",
         "API キーの欄が空です。「元に戻す」で既定のキーに戻すか、自分のキーを入れてください"
         "（2.3 の「ネットにつながっているとき」）。"],
        ["exe の画面が開かず、箱が出る",
         "DirectX 12 → OpenGL → ソフトウェア描画の順に試して、どれでも開けなかったときに"
         "理由を箱で出します。グラフィックスのドライバーを更新すると開くことがあります（3 章）。"],
        ["exe の図を動かすのが遅い・画面がおかしい",
         "描き方を替えて試せます（3 章の --glow・--gpu）。作者に知らせるときは、"
         "［解析］タブの「ログ」の 2 行目（「描画: …」）を写して添えてください。"],
        ["目標に矢印が向かない／位置がおかしい",
         "目標の名前や座標が間違っている可能性があります。"
         "画面に出る「画像中心」の座標が、撮ったつもりの場所と合っているか"
         "確かめてください。"],
        ["図の日本語が □□□ になる",
         "日本語フォントが見つかっていません。ふつうの Windows なら起きませんが、"
         "起きた場合はセル A の出力に「日本語フォントなし」と表示されます。"],
    ], st, widths=[52 * mm, CONTENT_W - 52 * mm]))

    add(PageBreak())

    # ================================================ 9. 入れ替え・削除 ===
    ext(h1("新しい版に入れ替える / やめる", st, "9"))

    ext(h2("新しい版に入れ替える", st, "9.1"))
    add(Paragraph("<b>［使い方］タブの「更新を確認」→「更新する」を押すだけ</b>です。"
                  "新しい版があるかを GitHub に聞き、あれば落として入れ替え、"
                  "起動し直します。", st.body))
    ext(fig("app-help.png", 150,
            "図 18　［使い方］タブ。更新のボタンは右上にあります。", st))
    ext(steps([
        "<b>［使い方］</b>タブを開き、<b>「更新を確認」</b>を押します。",
        "新しい版があれば、その版と大きさが出て"
        "<b>「更新する」</b>が押せるようになります。",
        "押すと、落とす → 中身を確かめる → 入れ替える → 起動し直す、"
        "まで自動で進みます。",
    ], st))
    add(Paragraph("落としたものは、入れ替える前に <b>SHA256 で中身を確かめます</b>。"
                  "違っていれば捨てて、入れ替えません。"
                  "いままでの版は入れ替えのあいだだけ <b>zahyou.exe.old</b> として横に残り、"
                  "<b>次に起動したときに消えます</b>"
                  "（v6 までは zahyou.exe.bak として残していました）。"
                  "前の版に戻したいときは、GitHub の Releases から落として上書きしてください。", st.body))
    add(Paragraph("起動したときにも自動で確認します（新しい版があるときだけ知らせます）。"
                  "確認をやめたいときは、同じところのチェックを外してください。",
                  st.small))
    ext(callout("手で入れ替えることもできます", [
        "GitHub の Releases（付録 B）から新しい zahyou.exe を落として、"
        "古いものに上書きするだけです。",
        "zahyou.exe を書き込めない場所（Program Files など）に置いていると"
        "ボタンでは入れ替えられません。その場合は、書ける場所へ移してください。",
    ], st, kind="note"))
    add(Paragraph("設定（目標・焦点距離・配色・API キー）も、覚えた座標や画素スケールも"
                  "そのまま引き継がれます。<b>星図データもそのままで構いません</b>。"
                  "v6 で方法を「オフライン (WSL) のみ」にしていたら、"
                  "v7 でも「オフラインのみ」として読みます"
                  "（設定ファイルには v6 と同じ書き方で書くので、v6 に戻しても方法は変わりません）。"
                  "v6 で WSL に入れた astrometry.net はもう使いません（消し方は 9.2）。",
                  st.body))
    ext(callout("v6 から入れ替える", [
        "<b>v6 の［使い方］タブの「更新を確認」→「更新する」で、v7 に入れ替わります。</b>",
        "v6 の exe は zahyou.exe.bak（約 102 MB）として横に残ります。"
        "v7 はこれを消さないので、v7 が動いたら消してかまいません。",
        "設定・覚えた座標・星図データは、そのまま使えます（WSL の astrometry.net が要らなくなることは上に書いたとおりです）。",
        "<b>v6 に戻すとき</b>は、v6 の zahyou.exe（zahyou.exe.bak を zahyou.exe に名前を戻したもの）を"
        "置くだけです。設定はそのまま使えます。"
        "API キーの欄の値は v6 では使わず、v6 で設定を保存すると消えます"
        "（次に v7 を開くと既定のキーに戻ります）。",
        "<b>β版（v7.0.0-beta.N）を使っていた方</b>は、β版の「更新を確認」で正式版が出ます。",
    ], st, kind="note"))
    add(Paragraph("いま使っている版は、<b>ウィンドウの題</b>で分かります"
                  "（「zahyou v7.0.0 — …」）。", st.body))
    ext(callout("ノートブック版は 2 つとも入れ替えてください", [
        "<b>zahyou_v7.ipynb</b> と <b>zahyou_engine.py</b> は組で動きます。"
        "片方だけ新しくすると、設定の名前が食い違って動かなくなることがあります。",
        "入れ替えたあとは、ノートブックを開き直して"
        "<b>セル A からやり直して</b>ください"
        "（開いたままだと、古いエンジンが読み込まれたまま残ります）。",
        "v6 のノートブック（zahyou_v6.ipynb）やβ版のノートブック（zahyou_v7_beta.ipynb）から移るときは、<b>zahyou_v7.ipynb</b> を開いてください（古いノートブックは消してかまいません）。",
    ], st))

    ext(h2("やめる（消す）", st, "9.2"))
    add(Paragraph("zahyou はインストールしないので、<b>置いたものを消すだけ</b>です。"
                  "使った場所は次の 4 つです。上から順に、消しても差し支えのない"
                  "ものから並べてあります。", st.body))
    ext(table([
        ["消すもの", "場所", "大きさ", "備考"],
        ["本体", "zahyou.exe を置いた場所", "約 13 MB", "削除するだけ"],
        ["設定と記憶", "%LOCALAPPDATA%\\zahyou<br/>"
         "（下の説明を見てください）", "数 KB",
         "設定と、覚えた座標"],
        ["星図データ", "C:\\AstrometryData", "数 GB",
         "オフラインで使った人だけ"],
        ["v6 の解析エンジン", "WSL（Ubuntu）の中", "約 200 MB",
         "v6 で入れた人だけ。下の手順で消します"],
    ], st, widths=[24 * mm, 46 * mm, 18 * mm, CONTENT_W - 88 * mm]))
    add(Paragraph("<b>%LOCALAPPDATA% は、Windows が「この PC の"
                  "この人の設定置き場」を表すために使う書き方</b>です。"
                  "実際には、次の場所を指しています。", st.body))
    ext(code_block([
        "%LOCALAPPDATA%\\zahyou",
        "",
        "  ↓ Windows に taro という名前で入っているなら",
        "",
        "C:\\Users\\taro\\AppData\\Local\\zahyou",
    ], st))
    ext(callout("フォルダーをたどっても見つかりません", [
        "<b>AppData は隠しフォルダー</b>なので、C:\\Users\\（名前）を"
        "開いても出てきません。次のどちらかで一発で開けます。",
        "① エクスプローラーの<b>アドレス欄</b>（いまの場所が出ている欄）を"
        "クリックして、<b>%LOCALAPPDATA%\\zahyou</b> と打って Enter。",
        "② <b>Windows キーを押しながら R</b> を押し、出てきた欄に"
        "同じものを打って Enter。",
        "自分の名前が分からないときは、エクスプローラーで C:\\Users を"
        "開いてください。並んでいるフォルダー名がそれです。",
        "デスクトップ版の［使い方］タブには、<b>その PC での実際の場所</b>が"
        "出ています。",
    ], st, kind="note"))

    add(Paragraph("v6 で WSL に入れた解析エンジン（astrometry.net）は、v7 では使いません。"
                  "残しておいても害はありません。片付けるなら、Ubuntu のターミナルで"
                  "<b>どちらか一方</b>を打ちます。", st.body))
    add(Paragraph("(1) astrometry.net を消す（設定ファイルも一緒に消え、"
                  "zahyou が残した退避ファイルも消します）", st.small))
    ext(code_block([
        "sudo apt remove --purge astrometry.net",
        "sudo rm /etc/astrometry.cfg.bak-zahyou",
    ], st))
    add(Paragraph("(2) astrometry.net はほかの用事に残し、"
                  "設定だけ zahyou が書き換える前に戻す", st.small))
    ext(code_block([
        "sudo cp /etc/astrometry.cfg.bak-zahyou /etc/astrometry.cfg",
        "sudo rm /etc/astrometry.cfg.bak-zahyou",
    ], st))
    add(Paragraph(".bak-zahyou は、v6 が /etc/astrometry.cfg を書き換える前に"
                  "自動で退避したものです。(1) の --purge は /etc/astrometry.cfg も消すので、"
                  "そのあとで退避ファイルを書き戻す必要はありません。", st.small))

    ext(callout("WSL 自体を消すかどうかは、よく確かめてから", [
        "WSL は zahyou 専用のものではありません。"
        "ほかの用事にも使っているなら、<b>消さないでください</b>。",
        "zahyou のためだけに入れたのであれば、"
        "PowerShell で次のように消せます。"
        "<b>1 行目は Ubuntu の中身をすべて消します（戻せません）。</b>",
    ], st))
    ext(code_block([
        "wsl --unregister Ubuntu",
        "wsl --uninstall",
    ], st))
    add(Paragraph("2 行目は Windows から WSL そのものを外します。"
                  "1 行目だけなら、あとで <font face=\'HackGen\'>wsl --install</font> "
                  "で入れ直せます。", st.small))
    add(Paragraph("ノートブック版を使っていた場合は、Python と VS Code も"
                  "「アプリと機能」から普通にアンインストールできます"
                  "（ほかで使っていないときだけ）。", st.body))

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
        ["WSL", "Windows の中で Linux を動かすしくみ。"
         "v6 まではオフライン解析に使っていました（v7 では要りません）。"],
        ["API キー", "nova.astrometry.net に問い合わせるときの合言葉。"
         "［解析］タブの欄に、初めから作者のキーが入っています。"
         "nova.astrometry.net に登録すると、自分のキーが自分のページで見られます。"],
        ["FITS", "天文でよく使う画像形式。撮影条件などの情報も一緒に入っています。"],
    ], st, widths=[32 * mm, CONTENT_W - 32 * mm]))

    ext(h1("参考リンクと出典", st, "付録 B"))
    add(Paragraph("配布ページ", st.h3))
    ext(bullets([
        "GitHub の Releases（最新版）："
        "https://github.com/Akashi3060/zahyou/releases/latest<br/>"
        "付けているもの: zahyou.exe（デスクトップ版）・zahyou_v7.ipynb と zahyou_engine.py"
        "（ノートブック版）・zahyou-manual-v4.pdf（このマニュアル）・LICENSE.txt・"
        "THIRD-PARTY-NOTICES.md（exe に入っているライブラリの一覧）・SHA256SUMS.txt（全部の SHA256）",
        "ソース：https://github.com/Akashi3060/zahyou",
        "配布ページ（zahyou.exe・ノートブック版・このマニュアルと、ノートブック版向けの星図データの zip）："
        "https://cloud.akashi-kosaku.uk/s/M2sNsnA2K4oJ4Co",
    ], st))
    add(Paragraph("星図データの原典", st.h3))
    ext(bullets([
        "5200 シリーズ（LITE）： "
        "https://portal.nersc.gov/project/cosmo/temp/dstn/index-5200/LITE/",
        "4100 シリーズ： https://data.astrometry.net/4100/",
    ], st))
    add(Paragraph("使用しているソフトウェア", st.h3))
    ext(bullets([
        "Astrometry.net（Lang et al. 2010, AJ 139, 1782）— 解き方の出典と、"
        "オンライン解析のサーバー",
        "デスクトップ版: egui・eframe / wgpu / tiny-skia / image / ureq・rustls / "
        "rayon / memmap2 ほか（Rust のクレート 213 個。一覧は THIRD-PARTY-NOTICES.md）",
        "ノートブック版: Astropy（座標と FITS の取り扱い）/ SciPy / NumPy / "
        "matplotlib / Pillow / reproject",
        "SIMBAD・VizieR（CDS, ストラスブール）— 天体名から座標を引くのに使用",
    ], st))

    add(Paragraph("ライセンス", st.h3))
    add(Paragraph("<b>zahyou 自身は MIT ライセンス</b>です。"
                  "自由に使って、書き換えて、配ってかまいません。"
                  "著作権表示だけ残してください。", st.body))
    ext(code_block([
        "Copyright (c) 2025-2026 Yoshitaka Fujikawa / AstroKIT,",
        "                        Kyushu Institute of Technology",
    ], st))
    ext(table([
        ["", "扱い"],
        ["zahyou のコード", "MIT"],
        ["zahyou.exe に入っている Rust のクレート（213 個）と画面の既定の書体",
         "MIT / Apache-2.0 / BSD / ISC / Zlib / SIL OFL など。"
         "<b>GPL・LGPL・MPL は含みません</b>"],
        ["astrometry.net",
         "<b>同梱も呼び出しもしていません</b>。解き方を論文と公開文書から"
         "書き直したもので、コードは含みません"],
        ["星図データ（index ファイル）",
         "<b>同梱していません</b>。実行時に原典から取得します。"
         "Tycho-2 / Gaia DR2 由来"],
    ], st, widths=[62 * mm, CONTENT_W - 62 * mm]))
    add(Paragraph("全文は配布ページと GitHub の Releases の <b>LICENSE.txt</b> と "
                  "<b>THIRD-PARTY-NOTICES.md</b> にあります。"
                  "後者は、exe に入る依存（cargo metadata）と各ライブラリの配布物から"
                  "機械的に作っていて、同じものが exe の［使い方］タブでも読めます。", st.small))

    add(HRule(CONTENT_W))
    add(Paragraph("このマニュアルの本文は Noto Sans JP（SIL Open Font License 1.1）、"
                  "コマンドの部分は HackGen で組んでいます。", st.small))

    doc.multiBuild(f)
    return out_path


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        HERE, "..", f"天体画像解析プログラム「zahyou」導入・運用マニュアル{VERSION}.pdf")
    p = build(os.path.abspath(out))
    print(f"wrote {p}  ({os.path.getsize(p)/1e6:.2f} MB)")
