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
-- the hanger is a rope with a catenary of its own (it sags when slack, it never pushes)
assert(get('hanger', 'sag') ~= nil, 'the hanger is not a cable')
-- joints and the wheel lose speed faster than the call's default (own damping columns)
assert(get('joint_3', 'damping') == 0.996 and get('wheel', 'damping') == 0.984, 'damping columns')
-- the road is a chain of ropes: no link carries a push (mode 1), the road sags between the fixings
assert(get('beam_1', 'mode') == 1, 'a road link is not a rope')
assert(get('joint_2', 'y') > 305, 'the road does not sag: ' .. get('joint_2', 'y'))
assert(get('joint_5', 'y') < get('joint_2', 'y'), 'the hanger does not lift the middle')
-- every part has a strength of its own, and cables tell their tension
assert(get('beam_1', 'strength') ~= get('beam_2', 'strength'), 'strengths are not individual')
assert(get('cable_a', 'tension') > 0 and get('hanger', 'strength') > 0, 'no tension or strength')
-- masses come from the beams: mass0 1 + half of two beams of 40.5 (density 1)
assert(math.abs(1 / get('joint_3', 'inv_mass') - 41.5) < 1e-6, 'joint mass: ' .. 1 / get('joint_3', 'inv_mass'))
-- a weak beam is judged in one tick and breaks in the next: two half beams and two new joints
-- replace it, the mass moves (a quarter of the beam to each new joint), the halves do not break
mouse(150, 250, false)
set('beam_5', 'strength', '1')
step(1)
assert(get('beam_5', 'doomed') == 1, 'the weak beam was not judged')
step(1)
assert(get('beam_5', 'rest_len') == nil, 'the broken beam is still there')
-- joint_4 now carries beam_4 / 2 and a half beam / 2: 1 + 20.25 + 10.125
assert(math.abs(1 / get('joint_4', 'inv_mass') - 31.375) < 1e-6, 'mass after the break: ' .. 1 / get('joint_4', 'inv_mass'))
step(300)
assert(get('joint_4', 'y') < 520 and get('hang', 'y') < 300, 'the bridge flew apart')
-- a rope breaks too and the halves hang
set('hanger', 'strength', '1')
step(2)
assert(get('hanger', 'rest_len') == nil, 'the broken hanger is still there')
step(120)
-- R restarts the level: the broken parts are back, the wheel is at its start
press('r')
step(1)
step(60)
assert(get('beam_5', 'rest_len') == 40.5 and get('hanger', 'rest_len') == 110, 'restart did not rebuild')
assert(get('beam_5', 'strength') > 1000000, 'restart: strength lost')
draw()
