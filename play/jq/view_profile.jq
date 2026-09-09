# Whole-trajectory scalar "profile": one compact line per round, so an agent can read the
# SHAPE of a long horizon without the verbose body. Near-zero per-round cost.
# Combine with an outer downsampler when the horizon is huge, e.g.:
#   jq -cs -f view_profile.jq | jq 'select(.round % 30 == 0)'
.[] | . as $s |
([ $s.cities[] | select(.razed == false) ] | length) as $tc |
{
  round: $s.round,
  cities: $tc,
  razed: ([ $s.cities[] | select(.razed == true) ] | length),
  ships: ($s.ships | length),
  fleet_hull: ([ $s.ships[].hull ] | add // 0),
  top_name: (
    [ $s.factions[] | . as $f | { name: $f.name, cities: ([ $s.cities[] | select(.faction_id == $f.id and .razed == false) ] | length) } ]
    | sort_by(-.cities) | .[0] | .name
  ),
  top_share: (
    [ $s.factions[] | . as $f | { name: $f.name, cities: ([ $s.cities[] | select(.faction_id == $f.id and .razed == false) ] | length) } ]
    | sort_by(-.cities) | .[0] | ( if $tc > 0 then .cities / $tc else 0 end )
  ),
  wars: (
    ( $s.factions | length ) as $n |
    [ range(0; $n) as $i | range(0; $n) as $j | select($i < $j) |
      ($s.factions[$i]) as $a | ($s.factions[$j]) as $b |
      select( ( $a.relations[ ($b.id | tostring) ] // 0 ) <= -20.0 ) | 1
    ] | length
  )
}
