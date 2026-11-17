-- the bridge holds, the one hanger carries load, the fixings stay, the wheel follows the mouse
mouse(200, 250, false)
step(120)
-- fixed points did not move
assert(get('road_l', 'x') == 100 and get('road_l', 'y') == 300, 'a fixing moved')
assert(get('cable_r', 'x') == 500 and get('cable_r', 'y') == 100, 'a fixing moved')
-- the hanger carries the road: its force is positive and the cable joint stays above the road
assert(get('hanger', 'force_max') > 0, 'the hanger carries nothing')
assert(get('hang', 'y') < get('joint_5', 'y') - 50, 'the cable joint fell onto the road')
-- the cable curves
assert(get('cable_a', 'catenary_a') ~= 0 and get('cable_a', 'sag') > 0, 'no catenary')
-- the wheel follows the mouse along the road
mouse(420, 250, false)
step(240)
local wx, wy = get('wheel', 'x'), get('wheel', 'y')
assert(math.abs(wx - 420) < 15, 'the wheel did not follow the mouse: x ' .. wx)
assert(wy < get('joint_5', 'y') and wy > 200, 'the wheel is not on the road: y ' .. wy)
mouse(150, 250, false)
step(240)
assert(math.abs(get('wheel', 'x') - 150) < 15, 'the wheel did not come back: x ' .. get('wheel', 'x'))
