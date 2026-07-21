# Vibra Pong Game (console version)
print "=== Vibra Pong ==="
print ""

var score_p1 = 0
var score_p2 = 0
var ball_x = 20
var ball_y = 10
var paddle1 = 10
var paddle2 = 10
var round = 0

var i = 0
while i < 15 {
    var j = 0
    while j < 40 {
        if i == ball_y && j == ball_x {
            print "O"
        } else if j == 1 && (i >= paddle1 && i < paddle1 + 3) {
            print "|"
        } else if j == 38 && (i >= paddle2 && i < paddle2 + 3) {
            print "|"
        } else if j == 19 || j == 20 {
            print ":"
        } else {
            print " "
        }
        j = j + 1
    }
    print ""
    i = i + 1
}

print ""
print "P1: " + score_p1 + "  P2: " + score_p2
print "Game simulation complete!"
