// @ts-nocheck — dialect async (resume-continuations) is not Promise-based
// async v2 — payable: the deposit argument flows THROUGH (the manual
// promise DAG carries it; V1's call-await was hardwired zero-deposit).
// attach() forwards the attached deposit to the token as an ftMint for
// the user, then the resume reports the balance.
const TOKEN = "tok3.v2.test.near";

export async function attach(user: string): string {
  const dep = near.attachedDepositU128();
  const res = await near.call(
    TOKEN,
    "ftMint",
    "{\"to\":\"" + user + "\",\"amount\":\"" + dep + "\"}",
    20000000000000,
    dep
  );
  if (res == "") {
    near.abort("mint failed");
  }
  return "attached:" + res;
}
