(memory 2)

(define OUTLAYER "outlayer.testnet")
(define GAS-OL 300000000000000)
(define SRC-PART1 "{\"WasmUrl\":{\"url\":\"https://test.fastfs.io/kampy.testnet/outlayer.testnet/0eeb1b8747d5080e1d69f26603c61576ddb51793323ca8f9b518eb28ebae116d.wasm\",\"hash\":\"0eeb1b8747d5080e1d69f26603c61576ddb51793323ca8f9b518eb28ebae116d\",\"build_target\":\"wasm32-wasip2\"}}")

(define (get_btc_price)
  (near/call OUTLAYER "request_execution" SRC-PART1 GAS-OL 0))

(export "get_btc_price" get_btc_price false)
