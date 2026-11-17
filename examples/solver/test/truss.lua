-- the truss: stands, the wheel rides on its road, nothing breaks under the wheel
press('2')
step(1)
assert(get('app', 'level') == 2, 'the level did not change')
step(120)
assert(get('post_5', 'strength') > 1000000, 'no truss')
assert(get('hanger', 'rest_len') == nil and get('beam_1', 'rest_len') == nil, 'the first bridge is still there')
-- a rigid truss hardly sags: the road middle is within 6 px of 300
assert(math.abs(get('b5', 'y') - 300) < 6, 'the truss sags: ' .. get('b5', 'y'))
-- the wheel rides on the road (the mouse 10 px under the road line presses it) and nothing breaks.
-- pulling the mouse 20 px or more under the road loads the middle members past their strength
mouse(300, 290, false)
step(240)
assert(math.abs(get('wheel', 'x') - 300) < 15, 'the wheel did not follow: ' .. get('wheel', 'x'))
assert(get('wheel', 'y') < 300 and get('wheel', 'y') > 270, 'the wheel is not on the road: ' .. get('wheel', 'y'))
assert(get('road_4', 'rest_len') ~= nil and get('diag_4', 'rest_len') ~= nil and get('top_4', 'rest_len') ~= nil, 'a member broke under the wheel')
draw()
