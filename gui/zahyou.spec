# -*- mode: python ; coding: utf-8 -*-
"""
PyInstaller の設定。build_exe.ps1 から呼ばれる。

    pyinstaller zahyou.spec --noconfirm

出来上がり:
    dist/zahyou/zahyou.exe   … 起動が速い (配るときはフォルダーごと zip)
    ZAHYOU_ONEFILE=1 を付けて建てると 1 ファイルの exe になる
    (起動のたびに数百 MB を展開するので、初回 20 秒ほどかかる)

要点が 2 つある。

1. 解析エンジン (zahyou_engine.py) は「実行時に exec して読む」ので、
   PyInstaller の静的解析では中の import が見えない。
   必要なものは hiddenimports に自分で並べる ―― 抜けると exe だけが落ちる。

2. collect_all("astropy") / collect_all("astroquery") は使わない。
   これらは「パッケージの全サブモジュール」を hiddenimports に足すので、
   テスト用・任意依存の import までたどってしまい、torch や cv2 まで
   引きずり込んで 4.4 GB になった (実測)。
   要るモジュールだけを名指しし、データファイルだけを collect_data_files で拾う。
"""
import os

from PyInstaller.utils.hooks import collect_data_files, collect_submodules

ONEFILE = os.environ.get("ZAHYOU_ONEFILE") == "1"
ROOT = os.path.abspath(os.path.join(SPECPATH, ".."))       # noqa: F821

# --- エンジン本体を同梱する (実行時に読む) ---------------------------------
datas = [(os.path.join(ROOT, "zahyou_engine.py"), ".")]

# --- データファイル (中身のテーブルや設定。コードではない) ------------------
#  photutils は astroquery.astrometry_net が読み込む。__init__.py が自分の
#  CITATION.rst を開くので、データを入れないとオンライン解析だけが
#  FileNotFoundError で落ちる (実際に踏んだ)。
for pkg in ("astropy", "astroquery", "astropy_iers_data", "reproject",
            "astropy_healpix", "erfa", "matplotlib", "dask", "photutils"):
    try:
        datas += collect_data_files(pkg)
    except Exception:
        pass

# --- 実行時に import されるが、静的解析では見えないもの --------------------
hiddenimports = [
    "numpy",
    "scipy", "scipy.ndimage", "scipy.special", "scipy.spatial",
    "PIL", "PIL.Image", "PIL.ImageOps",
    "astropy", "astropy.io.fits", "astropy.wcs", "astropy.wcs.utils",
    "astropy.stats", "astropy.visualization", "astropy.coordinates",
    "astropy.units", "astropy.table", "astropy.time", "astropy.utils.data",
    "astropy.utils.iers", "astropy.io.ascii", "astropy.io.votable",
    # 天体名の解決と、オンライン解析 (使うのはこの 3 つだけ)
    "astroquery", "astroquery.astrometry_net", "astroquery.vizier",
    "astroquery.simbad", "photutils",
    # reproject は import しただけで dask を要求する (北を上にした図で使う)
    "reproject", "reproject.interpolation", "reproject.adaptive",
    "dask", "dask.array", "cloudpickle", "toolz", "fsspec",
    "matplotlib", "matplotlib.pyplot",
    "matplotlib.backends.backend_agg", "matplotlib.backends.backend_tkagg",
]
# FITS の圧縮まわりは動的 import があるので、この配下だけは総ざらいする
hiddenimports += collect_submodules("astropy.io.fits")

# --- 要らないもの ---------------------------------------------------------
#  ここに書いたものは「解析には使わないと確かめたうえで」外している。
#  外し忘れると 4 GB 級になり、配れなくなる。
excludes = [
    # 機械学習系 (astropy/astroquery の任意依存からたぐられる)
    "torch", "torchvision", "torchaudio", "transformers", "tokenizers",
    "faiss", "faiss_cpu", "sentence_transformers", "safetensors",
    "huggingface_hub", "hf_xet", "onnxruntime", "tensorflow", "keras",
    "sklearn", "cv2", "skimage", "pyarrow", "numba", "llvmlite",
    # 大きいが解析では使わないもの
    "pandas", "h5py", "sympy", "bokeh", "plotly", "shapely",
    "mocpy", "cdshealpix", "regions", "yt_dlp", "gwcs", "asdf",
    # 別の GUI ツールキット / 開発用
    "PyQt5", "PyQt6", "PySide2", "PySide6", "wx", "gi",
    "IPython", "ipywidgets", "notebook", "jupyter", "nbformat", "nbclient",
    "pytest", "sphinx", "setuptools._distutils", "reportlab", "fontTools",
    "matplotlib.backends.backend_qtagg", "matplotlib.backends.backend_qt5agg",
    "matplotlib.backends.backend_webagg", "matplotlib.backends.backend_wx",
    "matplotlib.tests", "numpy.tests", "scipy.tests",
    # astropy.tests は外さないこと。astropy/__init__.py が
    # astropy.tests.runner を import するので、無いと astropy ごと落ちる。
]

a = Analysis(                                              # noqa: F821
    [os.path.join(SPECPATH, "zahyou_gui.py")],             # noqa: F821
    pathex=[SPECPATH],                                     # noqa: F821
    binaries=[],
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    runtime_hooks=[],
    excludes=excludes,
    noarchive=False,
)
pyz = PYZ(a.pure)                                          # noqa: F821

_icon = os.path.join(SPECPATH, "zahyou.ico")               # noqa: F821
_icon = _icon if os.path.exists(_icon) else None

if ONEFILE:
    exe = EXE(                                             # noqa: F821
        pyz, a.scripts, a.binaries, a.datas, [],
        name="zahyou",
        debug=False,
        bootloader_ignore_signals=False,
        strip=False,
        upx=False,
        console=False,                    # GUI なのでコンソールを出さない
        disable_windowed_traceback=False,
        icon=_icon,
    )
else:
    exe = EXE(                                             # noqa: F821
        pyz, a.scripts, [],
        exclude_binaries=True,
        name="zahyou",
        debug=False,
        strip=False,
        upx=False,
        console=False,
        disable_windowed_traceback=False,
        icon=_icon,
    )
    coll = COLLECT(                                        # noqa: F821
        exe, a.binaries, a.datas, strip=False, upx=False, name="zahyou")
