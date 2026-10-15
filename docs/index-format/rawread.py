"""Uncached read throughput (FILE_FLAG_NO_BUFFERING) to estimate cold-cache load time."""
import ctypes, ctypes.wintypes as W, sys, time, os
k32 = ctypes.WinDLL('kernel32', use_last_error=True)
CreateFileW = k32.CreateFileW; CreateFileW.restype = W.HANDLE
CreateFileW.argtypes = [W.LPCWSTR, W.DWORD, W.DWORD, W.LPVOID, W.DWORD, W.DWORD, W.HANDLE]
ReadFile = k32.ReadFile; ReadFile.argtypes = [W.HANDLE, W.LPVOID, W.DWORD, ctypes.POINTER(W.DWORD), W.LPVOID]
VirtualAlloc = k32.VirtualAlloc; VirtualAlloc.restype = W.LPVOID; VirtualAlloc.argtypes = [W.LPVOID, ctypes.c_size_t, W.DWORD, W.DWORD]
GENERIC_READ = 0x80000000; OPEN_EXISTING = 3; FILE_FLAG_NO_BUFFERING = 0x20000000; FILE_FLAG_SEQUENTIAL_SCAN = 0x08000000
CH = 8 * 2**20
buf = VirtualAlloc(None, CH, 0x3000, 0x04)
for p in sys.argv[1:]:
    h = CreateFileW(p, GENERIC_READ, 1, None, OPEN_EXISTING, FILE_FLAG_NO_BUFFERING | FILE_FLAG_SEQUENTIAL_SCAN, None)
    n = W.DWORD(); tot = 0; t0 = time.perf_counter()
    while True:
        ok = ReadFile(h, buf, CH, ctypes.byref(n), None)
        if not ok or n.value == 0: break
        tot += n.value
    dt = time.perf_counter() - t0; k32.CloseHandle(h)
    print('%s %.1f MiB in %.3fs = %.0f MiB/s (uncached)' % (os.path.basename(p), tot/2**20, dt, tot/2**20/dt))
