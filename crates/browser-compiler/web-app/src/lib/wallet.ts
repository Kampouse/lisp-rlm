import { NearConnector } from '@hot-labs/near-connect';

export type Network = 'testnet' | 'mainnet';

export interface WalletState {
  connected: boolean;
  accountId: string | null;
  network: Network;
}

export interface DeployResult {
  success: boolean;
  txHash: string | null;
  explorerUrl: string | null;
  error: string | null;
}

let connector: NearConnector | null = null;
let connectorNetwork: Network | null = null;
let currentAccountId: string | null = null;

export function getConnector(network: Network = 'testnet'): NearConnector {
  if (!connector || connectorNetwork !== network) {
    connector = new NearConnector({ network });
    connectorNetwork = network;
    connector.on('wallet:signIn', (t: any) => {
      currentAccountId = t.accounts?.[0]?.accountId ?? null;
    });
    connector.on('wallet:signOut', () => {
      currentAccountId = null;
    });
  }
  return connector;
}

export async function connectWallet(network: Network = 'testnet'): Promise<WalletState> {
  const conn = getConnector(network);

  // Try auto-connect first
  try {
    const wallet = await conn.wallet();
    const accounts = await wallet.getAccounts();
    if (accounts.length > 0) {
      currentAccountId = accounts[0].accountId;
      return { connected: true, accountId: currentAccountId, network };
    }
  } catch {
    // Not signed in yet
  }

  // Show wallet selector popup
  try {
    const wallet = await conn.connect();
    const accounts = await wallet.getAccounts();
    if (accounts.length > 0) {
      currentAccountId = accounts[0].accountId;
      return { connected: true, accountId: currentAccountId, network };
    }
  } catch (err: unknown) {
    console.error('Wallet connection failed:', err);
  }

  return { connected: false, accountId: null, network };
}

export async function disconnectWallet(): Promise<void> {
  const conn = getConnector();
  try {
    await conn.disconnect();
  } catch {
    // ignore
  }
  currentAccountId = null;
}

export function getWalletState(): WalletState {
  return {
    connected: currentAccountId !== null,
    accountId: currentAccountId,
    network: 'mainnet',
  };
}

/**
 * P1: Deploy compiled WASM as a NEAR smart contract.
 * Creates a sub-account and deploys the WASM code to it.
 */
export async function deployP1(
  wasmBytes: Uint8Array,
  contractName: string,
  network: Network = 'testnet',
): Promise<DeployResult> {
  if (!currentAccountId) {
    return { success: false, txHash: null, explorerUrl: null, error: 'Wallet not connected' };
  }

  const conn = getConnector(network);
  const subAccountId = `${contractName}.${currentAccountId}`;

  try {
    const wallet = await conn.wallet();
    const result = await wallet.signAndSendTransactions({
      transactions: [
        {
          receiverId: subAccountId,
          actions: [
            { type: 'CreateAccount' },
            { type: 'DeployContract' as const, params: { code: wasmBytes } },
          ],
        },
      ],
    });

    const txHash = result?.[0]?.transaction?.hash ?? null;
    return {
      success: true,
      txHash,
      explorerUrl: toExplorerUrl(txHash, network),
      error: null,
    };
  } catch (err: unknown) {
    const message = err instanceof Error ? err.message : String(err);
    return { success: false, txHash: null, explorerUrl: null, error: message };
  }
}

/**
 * P2: Submit compiled WASM to OutLayer contract for off-chain execution.
 */
export async function deployP2(
  wasmBytes: Uint8Array,
  outlayerContractId: string,
  network: Network = 'testnet',
): Promise<DeployResult> {
  if (!currentAccountId) {
    return { success: false, txHash: null, explorerUrl: null, error: 'Wallet not connected' };
  }

  const conn = getConnector(network);

  try {
    const wallet = await conn.wallet();
    const result = await wallet.signAndSendTransaction({
      receiverId: outlayerContractId,
      actions: [
        {
          type: 'FunctionCall' as const,
          params: {
            methodName: 'submit_binary',
            args: { code: btoa(String.fromCharCode(...new Uint8Array(wasmBytes))) },
            gas: '300000000000000',
            deposit: '0',
          },
        },
      ],
    });

    const txHash = result?.transaction?.hash ?? null;
    return {
      success: true,
      txHash,
      explorerUrl: toExplorerUrl(txHash, network),
      error: null,
    };
  } catch (err: unknown) {
    const message = err instanceof Error ? err.message : String(err);
    return { success: false, txHash: null, explorerUrl: null, error: message };
  }
}

export function toExplorerUrl(txHash: string | null, network: Network = 'testnet'): string | null {
  if (!txHash) return null;
  const prefix = network === 'mainnet' ? '' : `${network}.`;
  return `https://explorer.${prefix}near.org/transactions/${txHash}`;
}
