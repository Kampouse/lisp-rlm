// Price oracle — the YIELD pattern (NEAR PromiseYield):
//   requestPrice() suspends via yieldCreate("onPrice"): the callback runs
//   once NOW with a NotReady result (returns its pending path), then
//   RE-RUNS with the payload when the feeder calls yieldResume.
const GAS = 20000000000000;

export function requestPrice(): string {
  let p = near.yieldCreate("onPrice", "{}", GAS, 1);
  near.storageSet("yd", "yd:" + toStr(p));
  near.promiseReturn(p);
  return "requested:" + toStr(p);
}

export function onPrice(): string {
  let n = near.promiseResultsCount();
  if (n == 0 || near.promiseSucceeded(0) == 0) {
    // NotReady pass — real NEAR runs the callback optimistically
    return "pending";
  }
  let price = near.promiseResult(0);
  near.storageSet("price", price);
  return "priced:" + price;
}

export function getPrice(): string {
  return near.storageGet("price") ?? "unset";
}
