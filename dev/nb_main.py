# ============================================================== 実行本体 ===

def _pick_image_path():
    if IMAGE_PATH:
        if not os.path.exists(IMAGE_PATH):
            raise FileNotFoundError(f"IMAGE_PATH が見つかりません: {IMAGE_PATH}")
        return IMAGE_PATH, os.path.basename(IMAGE_PATH)
    picked = globals().get("ZAHYOU_PICKED") or {}
    if picked.get("path"):
        return picked["path"], picked.get("name") or os.path.basename(picked["path"])
    raise RuntimeError(
        "画像が選ばれていません。1 つ目のセルのボタンで選ぶか、"
        "IMAGE_PATH にファイルのパスを書いてください。")


def _resolve_target(online):
    """設定から目標の SkyCoord を作る。作れなければ None。"""
    # 完全ブラインド: 目標を決めずに、画像がどこを向いているかだけ知りたいとき。
    # 天体名が空のときも同じ扱いにする (打ち忘れで止まるより親切)。
    if INPUT_MODE == 'NONE' or (INPUT_MODE == 'STAR_NAME'
                                and not str(TARGET_STAR_NAME or '').strip()):
        _log("  目標は指定されていません。画像がどこを向いているかだけ求めます。")
        return None
    if INPUT_MODE == 'COORDS':
        try:
            return SkyCoord(RA_INPUT_STR, DEC_INPUT_STR, frame='icrs')
        except Exception as e:
            _log(f"❌ 赤経赤緯を読み取れません ({e})。書式を確認してください。")
            return None
    if INPUT_MODE == 'STAR_NAME':
        ra, dec = resolve_target_name(TARGET_STAR_NAME, online=online)
        if ra is not None:
            return SkyCoord(ra=ra * u.deg, dec=dec * u.deg, frame='icrs')
        if RA_INPUT_STR and DEC_INPUT_STR:
            try:
                c = SkyCoord(RA_INPUT_STR, DEC_INPUT_STR, frame='icrs')
                _log("  → 代わりに RA_INPUT_STR / DEC_INPUT_STR の座標を使います。")
                return c
            except Exception:
                pass
        return None
    _log(f"❌ INPUT_MODE が不正です: {INPUT_MODE!r} ('STAR_NAME' か 'COORDS')")
    return None


def _existing_wcs_header(header, shape):
    """画像に元から入っている WCS が使えるなら、その FITS ヘッダを返す。"""
    try:
        h = normalize_wcs_header(header, shape)
        w = WCS(h)
        if not w.is_celestial:
            return None
        s = float(np.mean(proj_plane_pixel_scales(w.celestial)) * 3600.0)
        return h if 0.01 < s < 3600 else None
    except Exception:
        return None


def _sanity_ok(wcs, shape):
    """解いた結果がまともかどうか、描画の前に確かめる。"""
    try:
        s = float(np.mean(proj_plane_pixel_scales(wcs.celestial)) * 3600.0)
        c = wcs.pixel_to_world(shape[1] / 2.0, shape[0] / 2.0)
        return (0.01 < s < 3600) and np.isfinite(c.ra.deg) and np.isfinite(c.dec.deg)
    except Exception as e:
        _log(f"  ⚠️ WCS の検証に失敗しました: {e}")
        return False


def run():
    t0 = time.time()
    img_path, img_name = _pick_image_path()
    _log("=" * 62)
    _log(f"  zahyou v6   {img_name}")
    _log("=" * 62)

    # ---------------------------------------------------------------- 画像 ---
    bundle = load_image_any(img_path)
    ny, nx = bundle.shape
    _log(f"読み込み : {nx} x {ny} 画素   [{bundle.note}]")

    # ------------------------------------------------------------ 星の検出 ---
    # オンラインでもオフラインでも同じ星のリストを使う。
    # v5 は 1〜99 パーセンタイルで 8bit PNG に変換していたため、星が軒並み
    # 真っ白に飽和し、ノイズと区別がつかなくなって解けなくなっていた。
    _log("\n星を検出しています...")
    sub, mask, sigma, info = preprocess(bundle.data)
    for line in info:
        _log(f"  - {line}")
    sources, thr = detect_sources(sub, mask, sigma)
    if sources:
        fwhm = float(np.median([s["fwhm"] for s in sources]))
        _log(f"  - 星を {len(sources)} 個検出 "
             f"(しきい値 {thr:g}σ / 典型 FWHM {fwhm:.1f} px)")
        if len(sources) < 8:
            _log("  ⚠️ 星が少なすぎます。露出を伸ばすか複数フレームを重ねてください。")
    else:
        _log("  ⚠️ 星を検出できませんでした。露出・ピント・雲を確認してください。")

    # ------------------------------------------------------------- つながり ---
    if SOLVE_MODE == 'ONLINE':
        online = True
    elif SOLVE_MODE == 'OFFLINE':
        online = False
    else:
        online = internet_available()
    _log("\n🌍 オンライン環境で実行します。" if online else "\n🔌 オフライン環境で実行します。")

    # ---------------------------------------------------------------- 目標 ---
    target = _resolve_target(online)
    hint = (target.ra.deg, target.dec.deg) if (target is not None and USE_TARGET_AS_HINT) else None
    scale_hint, scale_why = estimate_pixel_scale(bundle.header, FOCAL_LENGTH_MM,
                                                 bundle.shape)

    # ---------------------------------------------------------------- 解析 ---
    wcs_header, how = None, ""

    if not IGNORE_EXISTING_WCS:
        h = _existing_wcs_header(bundle.header, bundle.shape)
        if h is not None:
            _log("\n✅ 画像に入っていた WCS をそのまま使います。")
            wcs_header, how = h, "画像に入っていた WCS"

    work_dir = tempfile.mkdtemp(prefix="zahyou_solve_")
    try:
        if wcs_header is None and online:
            _log(f"\n[オンライン] nova.astrometry.net に問い合わせます "
                 f"(最大 {ONLINE_TIMEOUT} 秒)")
            if scale_hint:
                _log(f"  - 画素スケールの手がかり: {scale_hint:.3f}″/px ({scale_why})")
            if hint:
                _log(f"  - 目標の方向 (半径 {SEARCH_RADIUS_DEG:g}°) を手がかりにします。")
            try:
                wcs_header = solve_online(
                    img_path, API_KEY, sources=sources, shape=bundle.shape,
                    timeout=ONLINE_TIMEOUT, hint_radec=hint,
                    hint_radius_deg=SEARCH_RADIUS_DEG, scale_hint=scale_hint)
            except Exception as e:
                _log(f"⚠️ オンライン解析でエラー: {type(e).__name__}: {e}")
                wcs_header = None
            if wcs_header is not None:
                _log("✅ オンライン解析に成功しました。")
                how = "オンライン (nova.astrometry.net)"
            elif SOLVE_MODE == 'ONLINE':
                _log("SOLVE_MODE='ONLINE' なのでここで終了します。")
            else:
                _log("   → ローカル (WSL) の解析に切り替えます。")

        if wcs_header is None and SOLVE_MODE != 'ONLINE':
            _log("\n[オフライン] WSL の astrometry.net を確認しています...")
            diag = solver_diagnostics()
            for m in diag["messages"]:
                _log(f"⚠️ {m}")
            if diag["ok"]:
                _log(f"  - solve-field OK / index ファイル {len(diag['indexes'])} 個")
                wcs_header = solve_offline(
                    bundle, sources, work_dir, timeout=OFFLINE_TIMEOUT,
                    focal_length_mm=FOCAL_LENGTH_MM, hint_radec=hint,
                    hint_radius_deg=SEARCH_RADIUS_DEG, diagnostics=diag)
                if wcs_header is not None:
                    how = "オフライン (WSL の astrometry.net)"

        if wcs_header is None:
            _log("\n❌ 座標を特定できませんでした。")
            return None

        # ------------------------------------------------------------ 検算 ---
        wcs = WCS(normalize_wcs_header(wcs_header, bundle.shape))
        if not _sanity_ok(wcs, bundle.shape):
            _log("❌ 解析結果がおかしいので採用しません。")
            return None

        summary = print_solution(wcs, bundle.shape)
        _log(f"  解いた方法    : {how}")
        _log(f"  所要時間      : {time.time() - t0:.1f} 秒")

        # 次回この機材で撮った画像は、設定を書かなくても一発で絞り込める
        f_mm = implied_focal_length(bundle.header, summary["scale"])
        if f_mm:
            _log(f"  逆算した焦点距離: {f_mm:.0f} mm")
        if scale_hint is None:
            _log("  → この機材の画素スケールを記憶しました "
                 "(次からは画角の総当たりをしません)")
        remember_scale(bundle.header, bundle.shape, summary["scale"])

        # ------------------------------------------------------------ 描画 ---
        shown = sources if SHOW_DETECTED_SOURCES else None

        if target is None:
            _log("\n⚠️ 目標の座標が無いので、矢印は描きません。")
            _log("   オフラインで天体名を使いたいときは、一度オンラインで実行して"
                 "座標を覚えさせるか、INPUT_MODE='COORDS' にしてください。")
            plot_original(wcs, bundle.display, None, bundle.flip_y,
                          sources=shown, draw_arrow=False)
            return wcs

        tx, ty = wcs.world_to_pixel(target)
        if np.all(np.isfinite([tx, ty])) and 0 <= tx < nx and 0 <= ty < ny:
            _log(f"\n  目標は画面内です: ピクセル ({float(tx):.1f}, {float(ty):.1f})")
            if sources:
                d = [float(np.hypot(s["x"] - tx, s["y"] - ty)) for s in sources]
                i = int(np.argmin(d))
                near_px = max(4.0, 3.0 * float(np.median([s["fwhm"] for s in sources])))
                if d[i] <= near_px:
                    _log(f"  検出した星と {d[i]:.1f} px "
                         f"({d[i] * summary['scale']:.1f}″) で一致しました。")
                else:
                    _log("  目標そのものは、この検出しきい値では拾えていません "
                         "(暗い星なら正常です)。")
        else:
            _log("\n  目標は画面の外です。矢印の向きへ動かしてください。")

        plot_original(wcs, bundle.display, target, bundle.flip_y, sources=shown)
        plot_north_up(wcs, bundle.display, target)
        return wcs

    finally:
        shutil.rmtree(work_dir, ignore_errors=True)


_result_wcs = run()
