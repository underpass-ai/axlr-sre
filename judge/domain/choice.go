package domain

import "time"

// Choice is Jev's answer to one question: the option it chose, the
// probability it gave that option, what the call cost.
type Choice struct {
	Option      string
	Probability float64
	InputTokens int
	Took        time.Duration
}

// JevInputPricePerToken is TypeSafe's price of an input token for Jev
// (0.042 USD per million, docs.typesafe.ai/models, 30 Oct 2026); output
// is free.
const JevInputPricePerToken = 0.042 / 1e6

// CostUSD is what the call cost.
func (c Choice) CostUSD() float64 { return float64(c.InputTokens) * JevInputPricePerToken }
