package data_avail

import (
	"fmt"

	gsrpc "github.com/MiguelDD1/go-substrate-rpc-client"
	"github.com/MiguelDD1/go-substrate-rpc-client/rpc/author"
	"github.com/MiguelDD1/go-substrate-rpc-client/signature"
	"github.com/MiguelDD1/go-substrate-rpc-client/types"
)

type ExtrinsicUniqueID struct {
	Block types.Hash
	Index uint32
}

/// Signs and sends the call `c`
func SignAndSend(api *gsrpc.SubstrateAPI, meta *types.Metadata, who signature.KeyringPair, appId uint32, c types.Call) (*author.ExtrinsicStatusSubscription, error) {
	ext := types.NewExtrinsic(c)

	genesisHash, err := api.RPC.Chain.GetBlockHash(0)
	if err != nil {
		return nil, err
	}

	rv, err := api.RPC.State.GetRuntimeVersionLatest()
	if err != nil {
		return nil, err
	}

	// Get the nonce for Alice
	key, err := types.CreateStorageKey(meta, "System", "Account", who.PublicKey, nil)
	if err != nil {
		return nil, err
	}

	var accountInfo types.AccountInfo
	ok, err := api.RPC.State.GetStorageLatest(key, &accountInfo)
	if err != nil || !ok {
		return nil, err
	}

	nonce := uint32(accountInfo.Nonce)

	so := types.SignatureOptions{
		BlockHash:   genesisHash,
		Era:         types.ExtrinsicEra{IsMortalEra: false},
		GenesisHash: genesisHash,
		Nonce:       types.NewUCompactFromUInt(uint64(nonce)),
		SpecVersion: rv.SpecVersion,
		Tip:         types.NewUCompactFromUInt(0),
		AppId:       types.NewUCompactFromUInt(uint64(appId)),
	}

	// Sign the transaction using Alice's default account
	err = ext.Sign(who, so)
	if err != nil {
		return nil, err
	}

	// Do the transfer and track the actual status
	return api.RPC.Author.SubmitAndWatchExtrinsic(ext)
}

func SubmitData(api *gsrpc.SubstrateAPI, who signature.KeyringPair, appId uint32, data []byte) (*ExtrinsicUniqueID, error) {
	meta, err := api.RPC.State.GetMetadataLatest()
	if err != nil {
		return nil, err
	}

	// Create the submit call
	submitCall, err := types.NewCall(meta, "DataAvailability.submit_data", data)
	if err != nil {
		return nil, err
	}

	sub, err := SignAndSend(api, meta, who, appId, submitCall)
	if err != nil {
		return nil, err
	}
	defer sub.Unsubscribe()

	for {
		status := <-sub.Chan()
		fmt.Printf("Transaction status: %#v\n", status)

		if status.IsInBlock {
			fmt.Printf("Completed at block hash: %#x\n", status.AsInBlock)

			blockHash, err := types.NewHashFromHexString("0xabab")
			if err != nil {
				return nil, err
			}

			return &ExtrinsicUniqueID{
				Block: blockHash,
				Index: 0,
			}, nil
		}
	}

}
