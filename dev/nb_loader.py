# ==============================================================================
#  セル A : 準備 (解析エンジンの読み込み)
#
#  初回だけ次の行の # を外して実行してください (以降は # を付けたままで結構です)。
# !pip install astroquery astropy matplotlib Pillow reproject ipywidgets scipy
# ==============================================================================

import os
import sys

_ENGINE = "zahyou_engine.py"

# このノートブックと同じフォルダーを探す (VS Code / Jupyter / Colab のどれでも通るように)
_dirs = [os.getcwd()]
if "__vsc_ipynb_file__" in globals():
    _dirs.insert(0, os.path.dirname(globals()["__vsc_ipynb_file__"]))
_dirs += [p for p in sys.path[:4] if p]

_path = next((os.path.join(d, _ENGINE) for d in _dirs
              if d and os.path.exists(os.path.join(d, _ENGINE))), None)

if _path is None:
    raise SystemExit(
        f"{_ENGINE} が見つかりません。\n"
        f"  zahyou_v6.ipynb と {_ENGINE} を、同じフォルダーに置いてください。\n"
        f"  探した場所: {[d for d in _dirs if d]}")

try:
    # globals() を渡すので、セル C で書く設定 (INPUT_MODE など) もエンジンから見える
    with open(_path, encoding="utf-8") as _f:
        exec(compile(_f.read(), _path, "exec"), globals())
except ImportError as _e:
    raise SystemExit(
        f"必要なライブラリが足りません ({_e.name})。\n"
        "  このセルの 5 行目の # を外して、一度だけ実行してください。")

print(f"準備できました。  Python {sys.version.split()[0]}"
      f" / 図のフォント: {ZAHYOU_FONT or '（日本語フォントなし）'}")
print("次のセル B を実行して、画像を選んでください。")
