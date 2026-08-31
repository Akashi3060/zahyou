# ================================================================ 画像選択 ===

import matplotlib


def setup_japanese_font():
    """図の日本語が豆腐 (□□□) にならないようフォントを選ぶ。"""
    import matplotlib.font_manager as fm
    have = {f.name for f in fm.fontManager.ttflist}
    for name in ("Yu Gothic", "Meiryo", "MS Gothic", "Noto Sans CJK JP",
                 "IPAexGothic", "Hiragino Sans"):
        if name in have:
            matplotlib.rcParams["font.family"] = name
            matplotlib.rcParams["axes.unicode_minus"] = False
            return name
    return None


ZAHYOU_FONT = setup_japanese_font()

# 選ばれた画像はここに入る。セル C から参照される。
ZAHYOU_PICKED = {"path": None, "name": None}


def _widget_files(w):
    """ipywidgets 7 系 (dict) と 8 系 (tuple) の両方に対応する。"""
    v = w.value
    if not v:
        return []
    if isinstance(v, dict):                      # v7: {filename: {...}}
        return [{"name": k, "content": d["content"]} for k, d in v.items()]
    return [{"name": f["name"], "content": f["content"]} for f in v]


def pick_image():
    """
    画像を選ぶボタンを出す。選んだファイルは ZAHYOU_PICKED に入る。

    戻り値は None にしておくこと。ウィジェットを返すと、display() で出した分と
    「セルの最後の式の値」として自動表示される分の 2 つが並んでしまう。
    """
    import ipywidgets as widgets
    from IPython.display import display

    status = widgets.Output()
    uploader = widgets.FileUpload(
        accept=".fits,.fit,.fts,.jpg,.jpeg,.png,.tif,.tiff",
        multiple=False,
        description="画像ファイルを選択",
        button_style="primary",
        layout=widgets.Layout(width="260px"),
    )

    def on_upload(_change):
        files = _widget_files(uploader)
        if not files:
            return
        f = files[0]
        suffix = os.path.splitext(f["name"])[1] or ".fits"
        tmp = tempfile.NamedTemporaryFile(delete=False, suffix=suffix,
                                          prefix="zahyou_")
        tmp.write(bytes(f["content"]))
        tmp.close()
        ZAHYOU_PICKED["path"] = tmp.name
        ZAHYOU_PICKED["name"] = f["name"]
        uploader.button_style = "success"
        with status:
            status.clear_output(wait=True)
            print(f"✅ 選択しました: {f['name']}  ({len(f['content']) / 1024:.0f} KB)")

    uploader.observe(on_upload, names="value")
    display(widgets.VBox([uploader, status]))
    print("画像を選んだら、次のセル C を実行してください。")
    print("※ ボタンを使わず、セル C の IMAGE_PATH にパスを直接書いても構いません。")
    return None
