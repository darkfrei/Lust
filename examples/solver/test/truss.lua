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
-- the mouse goes down slowly (1 px per tick, as a hand does): a jump of 40 px at once throws the
-- wheel on the road with its whole momentum, and that hit breaks the truss (it should)
for y = 250, 290 do mouse(300, y, true) step(1) end
step(200)
assert(math.abs(get('wheel', 'x') - 300) < 15, 'the wheel did not follow: ' .. get('wheel', 'x'))
assert(get('wheel', 'y') < 300 and get('wheel', 'y') > 270, 'the wheel is not on the road: ' .. get('wheel', 'y'))
assert(get('road_4', 'rest_len') ~= nil and get('diag_4', 'rest_len') ~= nil and get('top_4', 'rest_len') ~= nil, 'a member broke under the wheel')
-- the buttons at the top right: a click on 'Bridge 1' goes back to the suspension bridge (the
-- button is let go first: a click is the button going down)
mouse(300, 290, false) step(1)
mouse(520, 20, true)
step(1)
mouse(520, 20, false)
step(2)
assert(get('app', 'level') == 1 and get('hanger', 'rest_len') == 110 and get('post_5', 'rest_len') == nil, 'the button did not change the bridge')
-- 'Bridge 2' and 'Restart'
mouse(620, 20, true)
step(1)
mouse(620, 20, false)
step(2)
assert(get('app', 'level') == 2 and get('post_5', 'rest_len') ~= nil, 'the Bridge 2 button did not work')
set('post_5', 'strength', '1')
step(40)
mouse(700, 20, true)
step(1)
mouse(700, 20, false)
step(2)
assert(get('post_5', 'rest_len') ~= nil and get('post_5', 'strength') > 1000000, 'the Restart button did not work')
-- the friction button steps 1, 25, 50, 100, 0, 1 %
local function click(x, y) mouse(x, y, true) step(1) mouse(x, y, false) step(1) end
assert(math.abs(get('app', 'dscale') - 0.01) < 1e-9, 'start friction')
click(560, 50)
assert(math.abs(get('app', 'dscale') - 0.25) < 1e-9, 'friction step: ' .. get('app', 'dscale'))
click(560, 50)
assert(math.abs(get('app', 'dscale') - 0.5) < 1e-9, 'friction step: ' .. get('app', 'dscale'))
press('f') step(1)
assert(math.abs(get('app', 'dscale') - 1) < 1e-9, 'friction key: ' .. get('app', 'dscale'))
press('f') step(1)
assert(get('app', 'dscale') == 0, 'friction zero')
-- the wheel let go (mouse button up) falls onto the road; D drives it to the right rolling (the
-- rim and the wheel move together: friction by coulomb with an inertia), without a key it rolls
-- on and slows, A drives it back
press('1') step(1)
mouse(200, 250, false)
step(120)
assert(get('wheel', 'contact') == 1, 'the free wheel is not on the road')
local x0 = get('wheel', 'x')
keys('d')
step(60)
local v = (get('wheel', 'x') - get('wheel', 'old_x')) * 960
assert(get('wheel', 'spin') > 10, 'the wheel does not turn: ' .. get('wheel', 'spin'))
assert(get('wheel', 'x') > x0 + 50, 'the wheel does not drive: ' .. get('wheel', 'x') - x0)
assert(math.abs(v - get('wheel', 'spin') * 12) < 0.5 * v, 'the wheel slips too much: ' .. v .. ' ' .. get('wheel', 'spin'))
keys('a')
step(90)
assert(get('wheel', 'spin') < 0 and get('wheel', 'old_x') > get('wheel', 'x'), 'A does not drive it back')
keys()
-- a click far from the wheel pulls it to the mouse through a damped spring: it may pass the mouse
-- a little, never by much (without the damper it swung past by 155 px)
press('1') step(1)
mouse(200, 250, false) step(120)
mouse(400, 200, true)
local most = 0
for i = 1, 120 do step(1) most = math.max(most, get('wheel', 'x')) end
assert(most < 425, 'the wheel swings past the mouse: ' .. most)
assert(math.abs(get('wheel', 'x') - 400) < 3 and math.abs(get('wheel', 'y') - 200) < 3, 'the wheel is not at the mouse')
mouse(400, 200, false) step(1)
draw()
