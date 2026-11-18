# Walks from waypoint to waypoint, pausing at each, facing the way it goes.

WAYPOINTS = [(4.0, 0.0), (4.0, -6.0), (-4.0, -6.0), (-4.0, 2.0)]

def on_begin(self):
    self.vars.target = 0
    self.vars.waited = 0.0

def on_anim(self, name):
    pass

def on_tick(self, dt):
    if self.vars.waited > 0.0:
        self.vars.waited -= dt
        self.walk = vec3(0, 0, 0)
        return
    x, z = WAYPOINTS[self.vars.target]
    to = vec3(x - self.position.x, 0, z - self.position.z)
    distance = sqrt(to.x * to.x + to.z * to.z)
    if distance < 0.4:
        self.vars.target = (self.vars.target + 1) % len(WAYPOINTS)
        self.vars.waited = self.pause
        self.walk = vec3(0, 0, 0)
        return
    self.yaw = atan2(-to.x, -to.z)
    self.walk = to * (self.speed / distance)
