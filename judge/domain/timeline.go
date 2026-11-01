package domain

import "time"

// Timeline is the incident events of one incident, oldest first.
type Timeline []Event

// Of keeps the events of one incident since a moment, in order.
func (t Timeline) Of(incidentID string, since time.Time) Timeline {
	var kept Timeline
	for _, e := range t {
		if e.IncidentID == incidentID && !e.CreatedAt.Before(since) {
			kept = append(kept, e)
		}
	}
	return kept
}

// OpenToSettled is the time from the last opening to the settling that
// follows it; zero when the timeline shows no such pair.
func (t Timeline) OpenToSettled() time.Duration {
	var opened time.Time
	for _, e := range t {
		switch {
		case e.Opens():
			opened = e.CreatedAt
		case e.Settles() && !opened.IsZero():
			return e.CreatedAt.Sub(opened)
		}
	}
	return 0
}

// Count is how many events of a type the timeline holds.
func (t Timeline) Count(kind string) int {
	n := 0
	for _, e := range t {
		if e.Type == kind {
			n++
		}
	}
	return n
}
