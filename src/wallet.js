const { Connection, Keypair, Transaction, SystemProgram, PublicKey } = require('@solana/web3.js');
const fs = require('fs');
const { Program, AnchorProvider, web3 } = require('@project-serum/anchor');
const idl = require('./xeris_stake_idl.json');

async function getConnection(bootstrapIps) {
    for (const ip of bootstrapIps.split(',')) {
        try {
            const conn = new Connection(`http://${ip}:8899`, 'confirmed');
            await conn.getVersion();
            return conn;
        } catch (e) {
            console.error(`Node ${ip} unavailable: ${e.message}`);
        }
    }
    throw new Error('No nodes available');
}

async function sendXRS(bootstrapIps, fromKeypairPath, toPubkeyStr, amount) {
    const connection = await getConnection(bootstrapIps);
    const from = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync(fromKeypairPath, 'utf8'))));
    const balance = await connection.getBalance(from.publicKey);
    const lamports = amount * 1e9;
    const fee = lamports * 0.001; // 0.1% fee
    if (balance < lamports + fee) {
        throw new Error(`Insufficient balance: ${balance / 1e9} XRS, required: ${(lamports + fee) / 1e9} XRS`);
    }
    const to = new PublicKey(toPubkeyStr);
    const tx = new Transaction().add(
        SystemProgram.transfer({
            fromPubkey: from.publicKey,
            toPubkey: to,
            lamports
        })
    );
    const signature = await connection.sendTransaction(tx, [from]);
    await connection.confirmTransaction(signature);
    console.log(`Sent ${amount} XRS to ${toPubkeyStr}. Fee: ${fee / 1e9} XRS. Signature: ${signature}`);
}

async function stakeXRS(bootstrapIps, fromKeypairPath, amount) {
    const connection = await getConnection(bootstrapIps);
    const from = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync(fromKeypairPath, 'utf8'))));
    const provider = new AnchorProvider(connection, { publicKey: from.publicKey, signTransaction: async (tx) => tx.sign(from) }, {});
    const program = new Program(idl, 'XerisStakeProgram', provider);
    const [stakeAccount] = await web3.PublicKey.findProgramAddress(
        [Buffer.from('stake'), from.publicKey.toBuffer()],
        program.programId
    );
    await program.rpc.initializeStake(new anchor.BN(amount * 1e9), {
        accounts: {
            stakeAccount,
            owner: from.publicKey,
            systemProgram: web3.SystemProgram.programId
        },
        signers: [from]
    });
    console.log(`Staked ${amount} XRS`);
}

async function unstakeXRS(bootstrapIps, fromKeypairPath, amount) {
    const connection = await getConnection(bootstrapIps);
    const from = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync(fromKeypairPath, 'utf8'))));
    const provider = new AnchorProvider(connection, { publicKey: from.publicKey, signTransaction: async (tx) => tx.sign(from) }, {});
    const program = new Program(idl, 'XerisStakeProgram', provider);
    const [stakeAccount] = await web3.PublicKey.findProgramAddress(
        [Buffer.from('stake'), from.publicKey.toBuffer()],
        program.programId
    );
    await program.rpc.unstake(new anchor.BN(amount * 1e9), {
        accounts: {
            stakeAccount,
            owner: from.publicKey
        },
        signers: [from]
    });
    console.log(`Unstaked ${amount} XRS`);
}

async function claimAirdrop(bootstrapIps, address) {
    const connection = await getConnection(bootstrapIps);
    const response = await fetch(`http://${bootstrapIps.split(',')[0]}:8899/airdrop/${address}/1000000000000`, { method: 'POST' });
    const result = await response.json();
    if (result.error) {
        throw new Error(`Airdrop failed: ${result.error}`);
    }
    console.log('Claimed 1,000 XRS airdrop');
}

if (require.main === module) {
    const [, , command, bootstrapIps, ...args] = process.argv;
    if (command === 'send') {
        const [, fromKeypairPath, toPubkeyStr, amount] = args;
        sendXRS(bootstrapIps, fromKeypairPath, toPubkeyStr, parseFloat(amount)).catch(console.error);
    } else if (command === 'stake') {
        const [, fromKeypairPath, amount] = args;
        stakeXRS(bootstrapIps, fromKeypairPath, parseFloat(amount)).catch(console.error);
    } else if (command === 'unstake') {
        const [, fromKeypairPath, amount] = args;
        unstakeXRS(bootstrapIps, fromKeypairPath, parseFloat(amount)).catch(console.error);
    } else if (command === 'airdrop') {
        const [, address] = args;
        claimAirdrop(bootstrapIps, address).catch(console.error);
    } else {
        console.log('Usage: node wallet.js <command> <bootstrapIps> <args>');
        console.log('Commands: send <fromKeypairPath> <toPubkeyStr> <amount>');
        console.log('         stake <fromKeypairPath> <amount>');
        console.log('         unstake <fromKeypairPath> <amount>');
        console.log('         airdrop <address>');
    }
}