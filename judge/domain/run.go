package domain

import "sort"

// Run is every scenario's result and the totals a node's goals judge.
type Run struct {
	Results []Result
}

// NewRun orders the results by scenario name, so a run rebuilt from files
// reads the same as one made in order.
func NewRun(results []Result) Run {
	sorted := append([]Result(nil), results...)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i].Scenario < sorted[j].Scenario })
	return Run{Results: sorted}
}

// Fired is how many scenarios opened an incident.
func (r Run) Fired() int { return r.count(func(x Result) bool { return x.Fired() }) }

// Correct is how many causes Jev found fully stated.
func (r Run) Correct() int { return r.count(func(x Result) bool { return x.Outcome() == Correct }) }

// Partly is how many causes were partly stated.
func (r Run) Partly() int { return r.count(func(x Result) bool { return x.Outcome() == Partly }) }

// Duplicates is the duplicate incidents over the run.
func (r Run) Duplicates() int {
	n := 0
	for _, x := range r.Results {
		n += x.Duplicates
	}
	return n
}

// CostUSD is the bots' cost over the run, children included.
func (r Run) CostUSD() float64 {
	cost := 0.0
	for _, x := range r.Results {
		cost += x.CostUSD
	}
	return cost
}

// JevCostUSD is what judging cost.
func (r Run) JevCostUSD() float64 {
	cost := 0.0
	for _, x := range r.Results {
		cost += x.Score.JevCostUSD()
	}
	return cost
}

// Passes reports whether the run clears the goals: every scenario fired,
// enough causes correct, no duplicates, the run under budget.
func (r Run) Passes(goals Goals) bool {
	return r.Fired() >= goals.Scenarios && r.Correct() >= goals.Correct &&
		r.Duplicates() <= goals.Duplicates && r.CostUSD() < goals.CostPerRun
}

func (r Run) count(keep func(Result) bool) int {
	n := 0
	for _, x := range r.Results {
		if keep(x) {
			n++
		}
	}
	return n
}

// NodeRun is the part of a run one node's goals judge.
type NodeRun struct {
	Goals Goals
	Run   Run
}

// ByNode splits the run: the read node's scenarios against ReadNode, the
// write node's against WriteNode; a part without scenarios is left out.
func (r Run) ByNode() []NodeRun {
	var read, write []Result
	for _, x := range r.Results {
		if x.WriteNode {
			write = append(write, x)
		} else {
			read = append(read, x)
		}
	}
	var parts []NodeRun
	if len(read) > 0 {
		parts = append(parts, NodeRun{Goals: ReadNode, Run: NewRun(read)})
	}
	if len(write) > 0 {
		parts = append(parts, NodeRun{Goals: WriteNode, Run: NewRun(write)})
	}
	return parts
}

// PassesByNode reports whether every node's part clears its goals.
func (r Run) PassesByNode() bool {
	for _, part := range r.ByNode() {
		if !part.Run.Passes(part.Goals) {
			return false
		}
	}
	return true
}
