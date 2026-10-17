#!/usr/bin/env python3
"""fee-gen surgery 2/2: mint-finish snapshots + burnc accruef fold.
Anchors are text-identical to pre-feegen1 source (feegen1 didn't touch
mint-finish/burnc). Anchor-verified asserts before write."""
import sys

P = "lib/clmm_pool_v5.lisp"
src = open(P).read()
orig = src
fails = []

def sub(old, new, tag, count=1):
    global src
    n = src.count(old)
    if n != count:
        fails.append(f"{tag}: anchor x{n} (want {count})")
        return
    src = src.replace(old, new)
    print(f"ok: {tag}")

# ── 1. snapshot key helper + accruef fold, after p5-req-cap ──
sub("""(define (p5-req-cap req ekey)
  (let ((e (p5-bz ekey)))
    (if (< (li-cmp req e) 1) req e)))""",
"""(define (p5-req-cap req ekey)
  (let ((e (p5-bz ekey)))
    (if (< (li-cmp req e) 1) req e)))

;; fee-snapshot key: per (who,a,b,cell) growth checkpoint at mint.
;; Pre-fork positions (parity mint) never get one: reads are 0 and the
;; fold credits nothing until the position is re-minted after the fork.
(define (p5-fskey w a b i)
  (str-cat "6|FS" (str-cat "X:" (str-cat w (str-cat ":"
    (str-cat (to-string a) (str-cat ":" (str-cat (to-string b)
      (str-cat ":" (to-string i))))))))))

;; accruef fold (Aggregate_Accrual verbatim): owed_fee =
;; Σ_{i∈[a,b]} (FG_i − FS_i) × d >> 64 — per-cell floor ⇒ under-credit
;; only (the div_floor64 slack). Then FS_i ← FG_i for the remainder:
;; the residual position keeps earning from current growth. Known
;; v1 under-credit (pool-favoring, never over-pays): a top-up mint
;; resets the whole key's snapshots, forfeiting pre-top-up accrual.
(define (p5-feefold who a b d side)
  (let ((acc (loop ((i a) (s "0"))
               (if (> i b)
                   s
                   (let ((fk (str-cat "6|FG" (str-cat side
                               (str-cat ":" (to-string i))))))
                     (recur (+ i 1)
                       (li-add s (p5-mulshr64
                                   (li-sub (p5-bz fk)
                                           (p5-bz (p5-fskey who a b i)))
                                   d)))))))))
    (begin
      (loop ((i a))
        (if (> i b)
            0
            (begin
              (p5-put (p5-fskey who a b i)
                      (p5-bz (str-cat "6|FG" (str-cat side
                                (str-cat ":" (to-string i))))))
              (recur (+ i 1)))))
      acc)))""",
    "feefold-def")

# ── 2. mint-finish: snapshot FGX/FGY per cell + FSP marker ──
sub("""(define (p5-mint-finish sender d a b amt)
  (begin
    (p5-put (str-cat "6|PM:" sender) "0")
    ;; position books — pointwise LQ += d, POS += d, widen support
    (loop ((i a))
      (if (> i b)
          0
          (let ((k (p5-lqkey i)))
            (begin (p5-put k (li-add (p5-bz k) d)) (recur (+ i 1))))))
    (p5-put (p5-poskey sender a b)
            (li-add (p5-bz (p5-poskey sender a b)) d))""",
"""(define (p5-mint-finish sender d a b amt)
  (begin
    (p5-put (str-cat "6|PM:" sender) "0")
    ;; position books — pointwise LQ += d, POS += d, widen support
    (loop ((i a))
      (if (> i b)
          0
          (let ((k (p5-lqkey i)))
            (begin
              (p5-put k (li-add (p5-bz k) d))
              ;; fee checkpoints (both sides) at birth
              (p5-put (p5-fskey sender a b i) (p5-bz (str-cat "6|FGX:" i)))
              (p5-put (p5-fskeyy sender a b i) (p5-bz (str-cat "6|FGY:" i)))
              (recur (+ i 1)))))))
    (p5-put (p5-poskey sender a b)
            (li-add (p5-bz (p5-poskey sender a b)) d))
    ;; fee epoch marker (parity mints never write it ⇒ accrue nothing)
    (p5-put (str-cat "6|FSP:" (str-cat sender
              (str-cat ":" (str-cat (to-string a)
                (str-cat ":" (to-string b)))))) "1")""",
    "mint-finish-snapshots")

# ── 3. fskeyy helper (fskey is X-side; Y needs its own prefix) ──
sub("""(define (p5-fskey w a b i)
  (str-cat "6|FS" (str-cat "X:" (str-cat w (str-cat ":"
    (str-cat (to-string a) (str-cat ":" (str-cat (to-string b)
      (str-cat ":" (to-string i))))))))))""",
"""(define (p5-fskey w a b i)
  (p5-fsk "X" w a b i))
(define (p5-fskeyy w a b i)
  (p5-fsk "Y" w a b i))
(define (p5-fsk s w a b i)
  (str-cat "6|FS" (str-cat s (str-cat ":" (str-cat w (str-cat ":"
    (str-cat (to-string a) (str-cat ":" (str-cat (to-string b)
      (str-cat ":" (to-string i)))))))))))""",
    "fskey-pair")

# ── 4. feefold must use the side-specific key too ──
sub("""                   (let ((fk (str-cat "6|FG" (str-cat side
                               (str-cat ":" (to-string i))))))
                     (recur (+ i 1)
                       (li-add s (p5-mulshr64
                                   (li-sub (p5-bz fk)
                                           (p5-bz (p5-fskey who a b i)))
                                   d)))))))))""",
"""                   (recur (+ i 1)
                     (li-add s (p5-mulshr64
                                 (li-sub (p5-bz (str-cat "6|FG" (str-cat side
                                            (str-cat ":" (to-string i)))))
                                         (p5-bz (p5-fsk side who a b i)))
                                 d)))))))))""",
    "feefold-key-side")

sub("""              (p5-put (p5-fskey who a b i)
                      (p5-bz (str-cat "6|FG" (str-cat side
                                (str-cat ":" (to-string i))))))
              (recur (+ i 1)))))""",
"""              (p5-put (p5-fsk side who a b i)
                      (p5-bz (str-cat "6|FG" (str-cat side
                                (str-cat ":" (to-string i))))))
              (recur (+ i 1)))))""",
    "feefold-reset-side")

# ── 5. burnc: fee fold into owed (capped at float), FWX/FWY audit keys ──
sub("""        ;; owed = the required amount, capped at the escrow float
        (let ((cx (p5-req-cap rx "6|ESCX")))
        (let ((cy (p5-req-cap ry "6|ESCY")))
        (let ((kx (str-cat "6|OWX:" who))
              (ky (str-cat "6|OWY:" who)))
          (begin
            (p5-put kx (li-add (p5-bz kx) cx))
            (p5-put ky (li-add (p5-bz ky) cy))
            (str-cat "burnc-ok:" (str-cat cx (str-cat ":" cy))))))))))))))))""",
"""        ;; fee fold (accruef) — LP trading fees for the burned span,
        ;; capped at the float; raw folds persist in 6|FWX/6|FWY.
        (let ((fx (p5-feefold who a b d "X")))
        (let ((fy (p5-feefold who a b d "Y")))
        (begin
          (p5-put "6|FWX" fx)
          (p5-put "6|FWY" fy)
        ;; owed = required + fees, each capped at the escrow float
        (let ((cx (p5-req-cap (li-add rx fx) "6|ESCX")))
        (let ((cy (p5-req-cap (li-add ry fy) "6|ESCY")))
        (let ((kx (str-cat "6|OWX:" who))
              (ky (str-cat "6|OWY:" who)))
          (begin
            (p5-put kx (li-add (p5-bz kx) cx))
            (p5-put ky (li-add (p5-bz ky) cy))
            (str-cat "burnc-ok:" (str-cat cx (str-cat ":" cy))))))))))))))))))""",
    "burnc-feefold")

if fails:
    print("FAILED ANCHORS:")
    [print(" ", f) for f in fails]
    sys.exit(1)

open(P, "w").write(src)
print(f"written: {len(src)} bytes (was {len(orig)})")
