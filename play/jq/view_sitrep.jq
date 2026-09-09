# Semantic sitrep: collapse one verbose per-round snapshot into a compact intent-shaped
# strategic overview for the agent. Safe to reuse; no Rust change. war_threshold is the
# config.combat.war_threshold value (= -20.0 in config/game.ron).
#
# Usage:  jq -s -f view_sitrep.jq traj.jsonl      (renders the LAST line)
#         jq -cs '.[] | ...' to loop per round.
.[-1] as $s |
($s.factions) as $fs |
($fs | length) as $n |
([ $s.cities[] | select(.razed == false) ] | length) as $total_cities |
{
  round: $s.round,
  time_month: $s.time_month,
  world: {
    cities:      $total_cities,
    razed:       ([ $s.cities[] | select(.razed == true)  ] | length),
    ships:       ($s.ships | length),
    fleet_hull:  ([ $s.ships[].hull ] | add // 0)
  },
  # Active war pairs = faction relation <= war_threshold (infer the `wars` list the view omits).
  wars: (
    [ range(0; $n) as $i | range(0; $n) as $j | select($i < $j) |
      ($fs[$i]) as $a | ($fs[$j]) as $b |
      ( $a.relations[ ($b.id | tostring) ] // 0 ) as $rel |
      select( $rel <= -20.0 ) |
      { a: $a.name, b: $b.name, rel: $rel }
    ]
  ),
  # Per-faction strategic columns.
  factions: (
    [ $fs[] | . as $f |
      {
        id: $f.id,
        name: $f.name,
        cities:      ([ $s.cities[] | select(.faction_id == $f.id and .razed == false) ] | length),
        ships:       ([ $s.ships[]  | select(.faction_id == $f.id) ] | length),
        fleet_hull:  ([ $s.ships[]  | select(.faction_id == $f.id) | .hull ] | add // 0),
        stockpile_units: ( [ $f.resources | to_entries[].value ] | add // 0 )
      }
    ]
  ),
  # Reading name of the leading faction + its standing share.
  top: (
    [ $s.factions[] | . as $f | { name: $f.name, cities: ([ $s.cities[] | select(.faction_id == $f.id and .razed == false) ] | length) } ]
    | sort_by(-.cities) | .[0] as $lead |
    { name: $lead.name, cities: $lead.cities, share: ( if $total_cities > 0 then ($lead.cities / $total_cities) else 0 end ) }
  )
}
