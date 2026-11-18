# Walks where the player faces, turns with the mouse, and uses what is near. Physics does the
# walking: this says which way.

def on_tick(self, dt):
    self.yaw += input("turn") * dt
    speed = self.run_speed if held("run") else self.walk_speed
    ahead = vec3(-sin(self.yaw), 0, -cos(self.yaw))
    right = vec3(cos(self.yaw), 0, -sin(self.yaw))
    self.walk = (ahead * input("forward") + right * input("right")) * speed
    if pressed("interact"):
        send(None, "interact", self.position)
