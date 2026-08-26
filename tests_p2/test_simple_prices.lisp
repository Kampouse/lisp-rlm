;; Simple prices fetch — only 2-arg str-cat for compatibility
(let* ((nbtc-p (str-cat (get (http-get "https://api.rhea.finance/list-token-price") "nbtc.bridge.near") "price"))
       (zec-p (str-cat (get (http-get "https://api.rhea.finance/list-token-price") "zec.omft.near") "price"))
       (usdt-p (str-cat (get (http-get "https://api.rhea.finance/list-token-price") "usdt.tether-token.near") "price")))
  (str-cat (str-cat "{\"nbtc.bridge.near\":\"" nbtc-p) (str-cat "\",\"zec.omft.near\":\"" zec-p)))
