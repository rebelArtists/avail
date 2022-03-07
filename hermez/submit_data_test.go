package data_avail

import (
	"fmt"
	"testing"

	gsrpc "github.com/MiguelDD1/go-substrate-rpc-client"
	"github.com/MiguelDD1/go-substrate-rpc-client/signature"
	"github.com/MiguelDD1/go-substrate-rpc-client/types"
	"github.com/stretchr/testify/assert"
)

func TestSubmitData(t *testing.T) {
	// Display the events that occur during a transfer by sending a value to bob

	// Instantiate the API
	// api, err := gsrpc.NewSubstrateAPI("wss://polygon-da-explorer.matic.today/ws")
	api, err := gsrpc.NewSubstrateAPI("ws://127.0.0.1:9944")
	assert.NoError(t, err)

	data := types.NewBytes(types.MustHexDecodeString("0xab1234"))

	txId, err := SubmitData(api, signature.TestKeyringPairAlice, 0, data)
	assert.NoError(t, err)

	fmt.Print("TxId: ", txId.Block, "-", txId.Index, "\n")
}
