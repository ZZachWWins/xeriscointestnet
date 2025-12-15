import React, { useState, useEffect } from 'react';
import { Connection, Keypair, Transaction, SystemProgram, PublicKey } from '@solana/web3.js';
import Chart from 'chart.js/auto';
import * as anchor from '@project-serum/anchor';
import { Program, AnchorProvider, web3 } from '@project-serum/anchor';
import idl from './xeris_stake_idl.json'; // Assume IDL for staking.rs

const Wallet = () => {
    const [balance, setBalance] = useState(0);
    const [stake, setStake] = useState(0);
    const [address, setAddress] = useState('');
    const [recipient, setRecipient] = useState('');
    const [amount, setAmount] = useState('');
    const [stakeAmount, setStakeAmount] = useState('');
    const [connection, setConnection] = useState(null);
    const [provider, setProvider] = useState(null);
    const [program, setProgram] = useState(null);
    const [nodes] = useState(['http://192.168.1.100:8899', 'http://192.168.1.101:8899']);
    const [fee, setFee] = useState(0);

    useEffect(() => {
        const init = async () => {
            // Try multiple nodes for redundancy
            let conn;
            for (const node of nodes) {
                try {
                    conn = new Connection(node, 'confirmed');
                    await conn.getVersion();
                    break;
                } catch (e) {
                    console.error(`Node ${node} unavailable: ${e.message}`);
                }
            }
            if (!conn) {
                alert('No nodes available');
                return;
            }
            setConnection(conn);

            // Initialize Anchor provider
            const provider = new AnchorProvider(conn, window.solana, {});
            setProvider(provider);
            const program = new Program(idl, 'XerisStakeProgram', provider);
            setProgram(program);

            // Load keypair from secure storage (macOS Keychain/Windows Credential Manager)
            try {
                const keypair = await loadKeypair(); // Implement secure storage
                setAddress(keypair.publicKey.toString());
                const balance = await conn.getBalance(keypair.publicKey);
                setBalance(balance / 1e9);

                // Fetch stake
                const [stakeAccount] = await web3.PublicKey.findProgramAddress(
                    [Buffer.from('stake'), keypair.publicKey.toBuffer()],
                    program.programId
                );
                try {
                    const account = await program.account.stakeAccount.fetch(stakeAccount);
                    setStake(account.amount / 1e9);
                } catch (e) {
                    setStake(0); // No stake account yet
                }
            } catch (e) {
                console.error('Keypair load failed:', e);
            }

            // Tokenomics chart
            const ctxTokenomics = document.getElementById('tokenomicsChart').getContext('2d');
            new Chart(ctxTokenomics, {
                type: 'line',
                data: {
                    labels: ['0', '730K', '1.46M', '2.19M', '2.92M', '13.87M'],
                    datasets: [{
                        label: 'Cumulative XRS Supply (Millions)',
                        data: [200, 450.0525, 575.07875, 637.591875, 668.8484375, 700],
                        borderColor: '#1E90FF',
                        backgroundColor: 'rgba(30, 144, 255, 0.2)',
                        fill: true,
                        tension: 0.4
                    }]
                },
                options: {
                    scales: {
                        x: { title: { display: true, text: 'Blocks Mined (Millions)' } },
                        y: { title: { display: true, text: 'Supply (Millions XRS)' }, beginAtZero: true, max: 800 }
                    },
                    plugins: { title: { display: true, text: 'XRS Tokenomics: 700M Capped Supply (~46 Years, $50M by 2025)' }, legend: { display: true } }
                }
            });

            // Market cap chart
            const ctxMarketCap = document.getElementById('marketCapChart').getContext('2d');
            new Chart(ctxMarketCap, {
                type: 'line',
                data: {
                    labels: ['Sep 2025', 'Oct 2025', 'Nov 2025', 'Dec 2025'],
                    datasets: [{
                        label: 'Market Cap (Millions USD)',
                        data: [0, 10, 25, 50],
                        borderColor: '#FFD700',
                        backgroundColor: 'rgba(255, 215, 0, 0.2)',
                        fill: true,
                        tension: 0.4
                    }]
                },
                options: {
                    scales: {
                        x: { title: { display: true, text: 'Time' } },
                        y: { title: { display: true, text: 'Market Cap (Millions USD)' }, beginAtZero: true, max: 60 }
                    },
                    plugins: { title: { display: true, text: 'XRS Market Cap Growth: $50M by Dec 2025' }, legend: { display: true } }
                }
            });
        };
        init();
    }, []);

    const loadKeypair = async () => {
        // Stub: Implement secure storage (e.g., macOS Keychain, Windows Credential Manager)
        // For now, use localStorage as a fallback
        const keypairJson = localStorage.getItem('keypair');
        if (!keypairJson) throw new Error('Keypair not found');
        return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(keypairJson)));
    };

    const sendXRS = async () => {
        if (!connection || !provider || !address || !recipient || !amount) {
            alert('Missing required fields');
            return;
        }
        try {
            const from = await loadKeypair();
            const to = new PublicKey(recipient);
            const lamports = parseFloat(amount) * 1e9;
            const feeLamports = lamports * 0.001; // 0.1% fee
            setFee(feeLamports / 1e9);
            if (balance < (lamports + feeLamports) / 1e9) {
                alert('Insufficient balance');
                return;
            }
            const tx = new Transaction().add(
                SystemProgram.transfer({
                    fromPubkey: from.publicKey,
                    toPubkey: to,
                    lamports
                })
            );
            const signature = await connection.sendTransaction(tx, [from]);
            await connection.confirmTransaction(signature);
            setBalance(balance - (lamports + feeLamports) / 1e9);
            alert(`Sent ${amount} XRS to ${recipient}. Fee: ${feeLamports / 1e9} XRS. Signature: ${signature}`);
        } catch (error) {
            alert(`Transaction failed: ${error.message}`);
        }
    };

    const stakeXRS = async () => {
        if (!program || !provider || !stakeAmount) {
            alert('Missing required fields');
            return;
        }
        try {
            const from = await loadKeypair();
            const [stakeAccount] = await web3.PublicKey.findProgramAddress(
                [Buffer.from('stake'), from.publicKey.toBuffer()],
                program.programId
            );
            const amount = parseFloat(stakeAmount) * 1e9;
            await program.rpc.initializeStake(new anchor.BN(amount), {
                accounts: {
                    stakeAccount,
                    owner: from.publicKey,
                    systemProgram: web3.SystemProgram.programId
                },
                signers: [from]
            });
            setStake(amount / 1e9);
            setBalance(balance - amount / 1e9);
            alert(`Staked ${stakeAmount} XRS`);
        } catch (error) {
            alert(`Staking failed: ${error.message}`);
        }
    };

    const unstakeXRS = async () => {
        if (!program || !provider || !stakeAmount) {
            alert('Missing required fields');
            return;
        }
        try {
            const from = await loadKeypair();
            const [stakeAccount] = await web3.PublicKey.findProgramAddress(
                [Buffer.from('stake'), from.publicKey.toBuffer()],
                program.programId
            );
            const amount = parseFloat(stakeAmount) * 1e9;
            await program.rpc.unstake(new anchor.BN(amount), {
                accounts: {
                    stakeAccount,
                    owner: from.publicKey
                },
                signers: [from]
            });
            setStake(stake - amount / 1e9);
            setBalance(balance + amount / 1e9);
            alert(`Unstaked ${stakeAmount} XRS`);
        } catch (error) {
            alert(`Unstaking failed: ${error.message}`);
        }
    };

    const claimAirdrop = async () => {
        if (!connection || !address) {
            alert('Missing required fields');
            return;
        }
        try {
            const from = await loadKeypair();
            const response = await fetch(`${nodes[0]}/airdrop/${address}/1000000000000`, { method: 'POST' });
            const result = await response.json();
            if (result.error) {
                alert(`Airdrop failed: ${result.error}`);
            } else {
                setBalance(balance + 1000); // 1,000 XRS
                alert('Claimed 1,000 XRS airdrop');
            }
        } catch (error) {
            alert(`Airdrop failed: ${error.message}`);
        }
    };

    return (
        <div>
            <h1>XerisCoin Wallet</h1>
            <p>Balance: {balance} XRS</p>
            <p>Staked: {stake} XRS</p>
            <input type="text" placeholder="Your Address" value={address} readOnly />
            <input type="text" placeholder="Recipient Address" value={recipient} onChange={(e) => setRecipient(e.target.value)} />
            <input type="number" placeholder="Amount (XRS)" value={amount} onChange={(e) => setAmount(e.target.value)} />
            <p>Fee: {fee} XRS</p>
            <button onClick={sendXRS}>Send XRS</button>
            <input type="number" placeholder="Stake Amount (XRS)" value={stakeAmount} onChange={(e) => setStakeAmount(e.target.value)} />
            <button onClick={stakeXRS}>Stake XRS</button>
            <button onClick={unstakeXRS}>Unstake XRS</button>
            <button onClick={claimAirdrop}>Claim Airdrop</button>
            <canvas id="tokenomicsChart" width="400" height="200"></canvas>
            <canvas id="marketCapChart" width="400" height="200"></canvas>
        </div>
    );
};

export default Wallet;