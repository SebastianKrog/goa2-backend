# Arena v2 anti-stall batch

The control combines probabilistic defense and discard with accumulated gold.
Each candidate uses that control until its stall clock reaches the listed
number of quiet turns. The clocks reset when their corresponding public
progress appears at a referee decision boundary.

Lane progress is a battle-zone shift or a change in wave counters. Combat
progress includes lane progress, a life counter lost, or an on-board minion
removed. Movement, card cycling and gold alone do not reset either clock.

| Candidate | Quiet turns | Clock | Change after activation |
| --- | ---: | --- | --- |
| lane advance | 4 | lane | Increase the payoff for advancing the active battle zone toward the enemy. |
| wave clear | 5 | lane | Reward clearing the last opposing minion when a friendly wave survives. |
| minion approach | 4 | combat | Close the distance to live enemy minions in the active battle zone. |
| hero threat | 5 | combat | Close to attack range against enemy heroes and their pieces. |
| crossfire | 6 | combat | Set up a second allied attacker on the same enemy minion. |
| next wave | 5 | lane | When ahead on minions, get an ally within reach of the next enemy spawn before pushing. |
| lane presence | 6 | lane | Favor a second allied hero in a still-contested battle zone. |
| life pressure | 6 | combat | Put extra value on taking enemy life counters. |
| attack ready | 4 | combat | Put heroes holding red or gold cards in range of an active enemy minion. |
| gold discount | 6 | lane | Reduce the gold weight from 0.10 to 0.035 while the lane is stalled. |

The three screening starts include the exact Forgotten Island stall
(Brynn and Bain vs. NebKher and Snorri) and two fresh map and roster combinations.
Both sides use the 1-second calibrated node caps. Each match report includes
the longest quiet stretch on each clock, first lane progress, and activation
counts. The win/loss screen and Python oracle verification still apply.
