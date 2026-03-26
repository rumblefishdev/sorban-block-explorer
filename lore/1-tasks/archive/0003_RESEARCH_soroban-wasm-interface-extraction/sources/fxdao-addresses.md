---
url: 'https://www.fxdao.io/docs/addresses/'
title: 'Addresses | FxDAO'
fetched_date: 2026-03-26
task_id: '0003'
image_count: 0
---

# Addresses | FxDAO

## Official addresses

> "Remember: if is not listed here, it's not from us."

The synthetic assets issued by the FxDAO protocol (and the governance asset) are classic Stellar assets, this means they have a classic account issuer just like all classic assets (USDC, AQUA, etc). It's done this way so all legacy wallets support our assets by default, we don't see any major reason why we should use native Soroban contracts for the Assets.

Here are the addresses:

## Testnet Addresses

### Accounts

- **Assets issuer:** GBBTSBSV55VUI6KQ32JARRR65HV7AJNOEZR7P5HLNT2EZIXHVDPDA5JW
- **Treasury account:** GAZ2HX5VEB5WUC7IDVKZYO3LZMANVEKEFZTLPOXA7LLZMVNJ4LU6MWKU
- **Admin account:** GA47KLJ7QIEK7R4IOMBINAJC7QFU7LSKFTQXIUOYJY4T5PNZSW676A3V
- **Oracle contract:** CCHXQJ5YDCIRGCBUTLC5BF2V2DKHULVPTQJGD4BAHW46JQWVRQNGA2LU
- **Protocol manager account:** GBBTSBSV55VUI6KQ32JARRR65HV7AJNOEZR7P5HLNT2EZIXHVDPDA5JW

### Contracts

**Assets Contracts:**

- **FXG contracts:** CB4WLX4IP2MWAT2ITRRO7I5YM743NILBBOWMUIVWYSLWWASZVRGB5YD3
- **USDx contracts:** CA2QJKOZF6WE3C45FCYDWB45337BKENLUU4EREWWXRIMHKWJSH6EEWVO
- **EURx contracts:** CBA2S6NROG4PN36FSFZTWGD4JVQDCUYBMCW2H4J64JCGH7ZSQYTAIZ54
- **GBPx contracts:** CDYP7LY3OIKHFVDID3CO6MQJ45T37N2G63NYXN33OQJPLW3X2PYRFHVT

**Protocol Contracts:**

- **Vaults contract:** CBUZ5NJKA5PRS4TBPHWMN4JGGRVIOQOKI4JUYLA2IXS3BEJKQKEWFW7D

## Mainnet Addresses

### Accounts

- **Assets issuer:** GAVH5ZWACAY2PHPUG4FL3LHHJIYIHOFPSIUGM2KHK25CJWXHAV6QKDMN
- **Treasury account:** GB4KOTOYRZA32BRBJUOYDCAUJNPG6RPNOZ7QYDC2WLPNM4KML4475CIV
- **Admin account:** GC7JVOXZJSHY3GHKWUJWKIUYWEJ4RZABSRQZQ5JBZEC5QUTYBUHVNIKV
- **Oracle contract:** CB5OTV4GV24T5USEZHFVYGC3F4A4MPUQ3LN56E76UK2IT7MJ6QXW4TFS
- **Protocol manager account:** GC7JVOXZJSHY3GHKWUJWKIUYWEJ4RZABSRQZQ5JBZEC5QUTYBUHVNIKV

### Contracts

**Assets Contracts:**

- **FXG contracts:** CDBR4FMYL5WPUDBIXTBEBU2AFEYTDLXVOTRZHXS3JC575C7ZQRKYZQ55
- **USDx contracts:** CDIKURWHYS4FFTR5KOQK6MBFZA2K3E26WGBQI6PXBYWZ4XIOPJHDFJKP
- **EURx contracts:** CBN3NCJSMOQTC6SPEYK3A44NU4VS3IPKTARJLI3Y77OH27EWBY36TP7U
- **GBPx contracts:** CBCO65UOWXY2GR66GOCMCN6IU3Y45TXCPBY3FLUNL4AOUMOCKVIVV6JC

**Protocol Contracts:**

- **Vaults contract:** CCUN4RXU5VNDHSF4S4RKV4ZJYMX2YWKOH6L4AKEKVNVDQ7HY5QIAO4UB
- **Locking Pool:** CDCART6WRSM2K4CKOAOB5YKUVBSJ6KLOVS7ZEJHA4OAQ2FXX7JOHLXIP

## Future situation

There are some accounts that will change once we are at phase 3 of the protocol, at that point the `Admin` and `treasury` accounts will get merged into the governance contract once the governance for the protocol is unlocked.

The oracle contract will be changed once there are more oracles that can serve us on the network, the Reflector team is already working on adding more quotes and we will be including them too so we use multiple oracles instead of just ours.
