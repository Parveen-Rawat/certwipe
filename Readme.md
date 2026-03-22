# certwipe

# CertWipe — Cryptographic Secure Erasure Tool

A CLI tool that solves the data remanence problem — when you 
delete a file, the data isn't actually gone. The pointer is 
removed but the data stays on disk, recoverable with the 
right tools and knowledge.

CertWipe makes deleted data cryptographically unrecoverable.

## How it works
1. Encrypts drive data using asymmetric cryptography
2. Generates a public/private key pair
3. Creates a tamper-evident certificate of erasure
4. Performs deletion
5. Destroys the private key
6. Result: data is forensically unrecoverable even with 
   hardware-level recovery tools

## Stack
Rust · Cryptography · PKI

## Status
MVP complete — built for Smart India Hackathon 2025

## Disclaimer
For authorized use only. Always ensure you have permission 
before wiping any data on any system.

## Author
Parveen Rawat (Sh2d0w)
github.com/Parveen-Rawat
