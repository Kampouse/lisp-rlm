// Oracle feeder — resumes a suspended yield with the payload.
export function feed(yid: string, price: string): string {
  let ok = near.yieldResume(yid, price);
  if (ok == 0) {
    near.abort("no such yield");
  }
  return "fed";
}
