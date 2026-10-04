#!/usr/bin/env python3
"""Builds crates/rechenkern/src/units/foods.txt from USDA FoodData Central: food name, grams in a US cup.

Usage: scripts/foods.py FoodData_Central_sr_legacy_food_csv_2018-04 > crates/rechenkern/src/units/foods.txt

- https://fdc.nal.usda.gov/fdc-datasets/FoodData_Central_sr_legacy_food_csv_2018-04.zip
  (SR Legacy, public domain under CC0 1.0)

Each food below names the SR Legacy food and the household measure to take its weight from
("cup, packed", "tbsp"...). Without a measure the cup is used, else the tablespoon, the
teaspoon or the fluid ounce. Names are what people write in recipes, lowercase; the first
word of a phrase decides nothing, longer names simply win.
"""

import csv
import sys
from decimal import Decimal

# How many of each measure make a cup.
PER_CUP = {"cup": 1, "tbsp": 16, "tsp": 48, "fl oz": 8}
PLAIN = ["cup", "cup (8 fl oz)", "cup 8 fl oz", "tbsp", "tsp", "fl oz"]

# Foods with a weight that isn't measured: water is a gram per milliliter.
FIXED = {"water, tap water": Decimal("236.5882365")}

# names | SR Legacy description | measure
FOODS = """
flour, all-purpose flour, all purpose flour, plain flour, white flour, wheat flour | Wheat flour, white, all-purpose, enriched, bleached |
bread flour, strong flour | Wheat flour, white, bread, enriched |
cake flour | Wheat flour, white, cake, enriched | cup unsifted, dipped
self-rising flour, self rising flour, self-raising flour, self raising flour | Wheat flour, white, all-purpose, self-rising, enriched |
whole wheat flour, whole-wheat flour, wholemeal flour, whole grain flour, whole-grain flour | Wheat flour, whole-grain (Includes foods for USDA's Food Distribution Program) |
rye flour, medium rye flour | Rye flour, medium |
dark rye flour | Rye flour, dark |
light rye flour | Rye flour, light |
buckwheat flour | Buckwheat flour, whole-groat |
rice flour, white rice flour | Rice flour, white, unenriched |
brown rice flour | Rice flour, brown |
corn flour, maize flour | Corn flour, whole-grain, yellow |
masa harina, masa | Corn flour, masa, enriched, white |
cornmeal, corn meal, polenta | Cornmeal, degermed, enriched, yellow |
cornstarch, corn starch, cornflour | Cornstarch |
potato flour | Potato flour |
chickpea flour, gram flour, besan | Chickpea flour (besan) |
soy flour | Soy flour, full-fat, raw | cup, stirred
oat flour | Oat flour, partially debranned |
barley flour | Barley flour or meal |
malt flour, barley malt flour | Barley malt flour |
millet flour | Millet flour |
sorghum flour | Sorghum flour, whole-grain |
arrowroot, arrowroot flour | Arrowroot flour |
carob flour, carob powder | Carob flour |
peanut flour | Peanut flour, defatted |
triticale flour | Triticale flour, whole-grain |
semolina | Semolina, enriched |
tapioca, tapioca pearls | Tapioca, pearl, dry |
wheat germ | Wheat germ, crude |
wheat bran, bran | Wheat bran, crude |
oat bran | Oat bran, raw |
rice bran | Rice bran, crude |
almond flour, almond meal, ground almonds | Nuts, almonds | cup, ground
bread crumbs, breadcrumbs, dry bread crumbs | Bread, crumbs, dry, grated, plain |
seasoned bread crumbs | Bread, crumbs, dry, grated, seasoned |
croutons | Croutons, plain |
graham cracker crumbs | Cookies, graham crackers, plain or honey (includes cinnamon) | cup, crushed

sugar, white sugar, granulated sugar, caster sugar, castor sugar | Sugars, granulated |
brown sugar, packed brown sugar, light brown sugar, dark brown sugar | Sugars, brown | cup packed
unpacked brown sugar | Sugars, brown | cup unpacked
powdered sugar, icing sugar, confectioners sugar, confectioner's sugar | Sugars, powdered | cup unsifted
sifted powdered sugar, sifted icing sugar | Sugars, powdered | cup sifted
raw sugar, turbinado sugar, demerara sugar | Sugar, turbinado |
honey | Honey |
maple syrup | Syrups, maple |
corn syrup, light corn syrup | Syrups, corn, light |
dark corn syrup | Syrups, corn, dark |
molasses, black treacle | Molasses |
agave, agave syrup, agave nectar | Sweetener, syrup, agave | cup
malt syrup, barley malt syrup | Syrups, malt |
sorghum syrup | Syrups, sorghum |
pancake syrup | Syrups, table blends, pancake |
chocolate syrup | Beverages, chocolate syrup |
caramel sauce | Toppings, butterscotch or caramel | tbsp
grenadine | Syrups, grenadine |

butter, salted butter, unsalted butter | Butter, without salt |
whipped butter | Butter, whipped, with salt |
ghee, clarified butter, butter oil | Butter oil, anhydrous |
margarine | Margarine, regular, 80% fat, composite, stick, with salt |
shortening, vegetable shortening | Shortening, vegetable, household, composite |
lard | Lard |
tallow, beef tallow, beef fat | Fat, beef tallow |
chicken fat, schmaltz | Fat, chicken |
goose fat | Fat, goose |
oil, vegetable oil, cooking oil, soybean oil | Oil, soybean, salad or cooking |
olive oil, extra virgin olive oil, extra-virgin olive oil | Oil, olive, salad or cooking |
canola oil, rapeseed oil | Oil, canola |
sunflower oil | Oil, sunflower, high oleic (70% and over) |
corn oil | Oil, corn, industrial and retail, all purpose salad or cooking |
peanut oil, groundnut oil | Oil, peanut, salad or cooking |
sesame oil | Oil, sesame, salad or cooking |
coconut oil | Oil, coconut |
avocado oil | Oil, avocado |
grapeseed oil | Oil, grapeseed |
palm oil | Oil, palm |
flaxseed oil, linseed oil | Oil, flaxseed, cold pressed |
walnut oil | Oil, walnut |
almond oil | Oil, almond |
hazelnut oil | Oil, hazelnut |
safflower oil | Oil, safflower, salad or cooking, high oleic (primary safflower oil of commerce) |
cottonseed oil | Oil, cottonseed, salad or cooking |
rice bran oil | Oil, rice bran |
mustard oil | Oil, mustard |
fish oil, cod liver oil | Fish oil, cod liver |
cocoa butter | Oil, cocoa butter |

milk, whole milk | Milk, whole, 3.25% milkfat, with added vitamin D |
reduced fat milk, reduced-fat milk | Milk, reduced fat, fluid, 2% milkfat, with added vitamin A and vitamin D |
low fat milk, low-fat milk, lowfat milk | Milk, lowfat, fluid, 1% milkfat, with added vitamin A and vitamin D |
skim milk, skimmed milk, nonfat milk, non fat milk, non-fat milk, fat free milk, fat-free milk | Milk, nonfat, fluid, with added vitamin A and vitamin D (fat free or skim) |
buttermilk, fluid buttermilk | Milk, buttermilk, fluid, cultured, lowfat |
dried buttermilk, buttermilk powder | Milk, buttermilk, dried | cup
powdered milk, milk powder, dry milk, nonfat dry milk | Milk, dry, nonfat, regular, without added vitamin A and vitamin D | cup
instant milk, dry instant milk, instant milk powder | Milk, dry, nonfat, instant, with added vitamin A and vitamin D | cup
whole milk powder, dry whole milk | Milk, dry, whole, with added vitamin D | cup
evaporated milk | Milk, canned, evaporated, with added vitamin D and without added vitamin A |
condensed milk, sweetened condensed milk | Milk, canned, condensed, sweetened |
goat milk, goat's milk | Milk, goat, fluid, with added vitamin D |
sheep milk, sheep's milk | Milk, sheep, fluid |
cream, heavy cream, heavy whipping cream, whipping cream, double cream | Cream, fluid, heavy whipping | cup, fluid (yields 2 cups whipped)
light whipping cream | Cream, fluid, light whipping | cup, fluid (yields 2 cups whipped)
light cream, single cream, coffee cream, table cream | Cream, fluid, light (coffee cream or table cream) |
whipped cream | Cream, fluid, heavy whipping | cup, whipped
half and half, half-and-half | Cream, fluid, half and half |
sour cream | Cream, sour, cultured |
yogurt, yoghurt, plain yogurt, natural yogurt | Yogurt, plain, whole milk | cup (8 fl oz)
cream cheese | Cheese, cream |
cottage cheese | Cheese, cottage, creamed, large or small curd | cup, small curd (not packed)
ricotta, ricotta cheese | Cheese, ricotta, whole milk |
parmesan, parmesan cheese, grated parmesan | Cheese, parmesan, grated |
cheddar, cheddar cheese, shredded cheddar, grated cheddar, shredded cheese, grated cheese | Cheese, cheddar (Includes foods for USDA's Food Distribution Program) | cup, shredded
mozzarella, mozzarella cheese, shredded mozzarella | Cheese, mozzarella, whole milk | cup, shredded
swiss cheese, shredded swiss cheese | Cheese, swiss | cup, shredded
monterey jack, monterey jack cheese | Cheese, monterey | cup, shredded
gruyere, gruyere cheese | Cheese, gruyere | cup, shredded
feta, feta cheese, crumbled feta | Cheese, feta | cup, crumbled
blue cheese, crumbled blue cheese | Cheese, blue | cup, crumbled, not packed
whey | Whey, sweet, fluid |
whey powder | Whey, sweet, dried | cup
eggnog | Eggnog |
ice cream, vanilla ice cream | Ice creams, vanilla, rich | cup
coconut milk | Nuts, coconut milk, canned (liquid expressed from grated meat and water) |
coconut cream | Nuts, coconut cream, raw (liquid expressed from grated meat) |
cream of coconut | Nuts, coconut cream, canned, sweetened |
coconut water | Nuts, coconut water (liquid from coconuts) |
soy milk, soymilk | Soymilk, original and vanilla, unfortified |
almond milk | Beverages, almond milk, sweetened, vanilla flavor, ready-to-drink |
rice milk | Beverages, rice milk, unsweetened | fl oz (approximate weight, 1 serving)

eggs, egg, beaten eggs, beaten egg, whole eggs | Egg, whole, raw, fresh | cup (4.86 large eggs)
egg whites, egg white | Egg, white, raw, fresh |
egg yolks, egg yolk | Egg, yolk, raw, fresh |
egg powder, powdered egg, dried egg | Egg, whole, dried | cup, sifted
egg white powder, dried egg white | Egg, white, dried, powder, stabilized, glucose reduced | cup, sifted

rice, white rice, long grain rice, long-grain rice, basmati rice, jasmine rice | Rice, white, long-grain, regular, raw, enriched |
medium grain rice, medium-grain rice, arborio rice | Rice, white, medium-grain, raw, enriched |
short grain rice, short-grain rice, sushi rice | Rice, white, short-grain, enriched, uncooked |
glutinous rice, sticky rice | Rice, white, glutinous, unenriched, uncooked |
cooked rice, cooked white rice | Rice, white, long-grain, regular, enriched, cooked |
instant rice | Rice, white, long-grain, precooked or instant, enriched, dry |
brown rice | Rice, brown, long-grain, raw (Includes foods for USDA's Food Distribution Program) |
cooked brown rice | Rice, brown, long-grain, cooked (Includes foods for USDA's Food Distribution Program) |
wild rice | Wild rice, raw |
cooked wild rice | Wild rice, cooked |
oats, rolled oats, oatmeal, quick oats, porridge oats, old-fashioned oats | Cereals, oats, regular and quick, not fortified, dry | cup
oat groats, whole oats | Oats (Includes foods for USDA's Food Distribution Program) |
quinoa | Quinoa, uncooked |
cooked quinoa | Quinoa, cooked |
couscous, cous cous | Couscous, dry |
cooked couscous | Couscous, cooked | cup, cooked
bulgur, bulgur wheat, bulghur, dry bulgur | Bulgur, dry |
cooked bulgur | Bulgur, cooked |
barley, pearl barley, pearled barley | Barley, pearled, raw |
hulled barley | Barley, hulled |
cooked barley | Barley, pearled, cooked |
millet | Millet, raw |
cooked millet | Millet, cooked |
puffed millet | Millet, puffed |
buckwheat, buckwheat groats | Buckwheat |
kasha, roasted buckwheat | Buckwheat groats, roasted, dry |
amaranth | Amaranth grain, uncooked |
rye, rye berries | Rye grain |
spelt | Spelt, uncooked |
kamut, khorasan wheat | Wheat, KAMUT khorasan, uncooked |
teff | Teff, uncooked |
sorghum | Sorghum grain |
triticale | Triticale |
wheat, wheat berries | Wheat, hard red winter |
sprouted wheat | Wheat, sprouted |
hominy | Hominy, canned, white |
macaroni, elbow macaroni | Pasta, dry, enriched | cup elbows
penne | Pasta, dry, enriched | cup penne
farfalle, bow tie pasta | Pasta, dry, enriched | cup farfalle
rotini, fusilli | Pasta, dry, enriched | cup rotini
pasta shells | Pasta, dry, enriched | cup shells
egg noodles | Noodles, egg, dry, enriched |

lentils, green lentils, brown lentils | Lentils, raw |
red lentils, pink lentils | Lentils, pink or red, raw |
cooked lentils | Lentils, mature seeds, cooked, boiled, without salt |
sprouted lentils | Lentils, sprouted, raw |
chickpeas, garbanzo beans, garbanzos | Chickpeas (garbanzo beans, bengal gram), mature seeds, raw |
cooked chickpeas | Chickpeas (garbanzo beans, bengal gram), mature seeds, cooked, boiled, without salt |
canned chickpeas | Chickpeas (garbanzo beans, bengal gram), mature seeds, canned, drained, rinsed in tap water | cup drained, rinsed
kidney beans, red kidney beans | Beans, kidney, all types, mature seeds, raw |
cooked kidney beans | Beans, kidney, all types, mature seeds, cooked, boiled, without salt |
black beans | Beans, black, mature seeds, raw |
cooked black beans | Beans, black, mature seeds, cooked, boiled, without salt |
pinto beans | Beans, pinto, mature seeds, raw (Includes foods for USDA's Food Distribution Program) |
cooked pinto beans | Beans, pinto, mature seeds, cooked, boiled, without salt |
navy beans, haricot beans | Beans, navy, mature seeds, raw |
great northern beans | Beans, great northern, mature seeds, raw (Includes foods for USDA's Food Distribution Program) |
lima beans, butter beans | Lima beans, large, mature seeds, raw |
mung beans | Mung beans, mature seeds, raw |
bean sprouts, mung bean sprouts, sprouted mung beans | Mung beans, mature seeds, sprouted, raw |
split peas, cooked split peas | Peas, split, mature seeds, cooked, boiled, without salt |
sprouted peas | Peas, mature seeds, sprouted, raw |
soybeans, soy beans | Soybeans, mature seeds, raw |
edamame | Edamame, frozen, unprepared |
peas, green peas | Peas, green, raw |
frozen peas | Peas, green, frozen, unprepared (Includes foods for USDA's Food Distribution Program) |
baked beans | Beans, baked, canned, plain or vegetarian |
hummus, houmous | Hummus, commercial |
tofu | Tofu, raw, firm, prepared with calcium sulfate | cup
tempeh | Tempeh |
miso | Miso |

almonds, almond, whole almonds | Nuts, almonds | cup, whole
sliced almonds, flaked almonds | Nuts, almonds | cup, sliced
slivered almonds | Nuts, almonds | cup, slivered
blanched almonds | Nuts, almonds, blanched | cup whole kernels
walnuts, walnut, chopped walnuts | Nuts, walnuts, english | cup, chopped
walnut halves | Nuts, walnuts, english | cup shelled (50 halves)
ground walnuts | Nuts, walnuts, english | cup, ground
black walnuts | Nuts, walnuts, black, dried | cup, chopped
pecans, pecan, chopped pecans | Nuts, pecans | cup, chopped
pecan halves | Nuts, pecans | cup, halves
hazelnuts, hazelnut, filberts | Nuts, hazelnuts or filberts | cup, whole
chopped hazelnuts | Nuts, hazelnuts or filberts | cup, chopped
ground hazelnuts | Nuts, hazelnuts or filberts | cup, ground
cashews, cashew nuts, cashew | Nuts, cashew nuts, dry roasted, without salt added | cup, halves and whole
peanuts, peanut | Peanuts, all types, raw |
roasted peanuts | Peanuts, all types, dry-roasted, without salt |
pistachios, pistachio nuts, pistachio | Nuts, pistachio nuts, raw |
macadamia nuts, macadamias | Nuts, macadamia nuts, raw | cup, whole or halves
brazil nuts | Nuts, brazilnuts, dried, unblanched | cup, whole
pine nuts, pignoli | Nuts, pine nuts, dried |
chestnuts | Nuts, chestnuts, european, roasted |
butternuts | Nuts, butternuts, dried |
coconut, shredded coconut, desiccated coconut | Nuts, coconut meat, dried (desiccated), sweetened, shredded | cup, shredded
flaked coconut, coconut flakes | Nuts, coconut meat, dried (desiccated), sweetened, flaked, packaged |
peanut butter, smooth peanut butter, creamy peanut butter | Peanut butter, smooth style, without salt |
chunky peanut butter, crunchy peanut butter | Peanut butter, chunk style, without salt |
almond butter | Nuts, almond butter, plain, without salt added |
cashew butter | Nuts, cashew butter, plain, without salt added |
sunflower seed butter | Seeds, sunflower seed butter, without salt |
tahini, sesame paste | Seeds, sesame butter, tahini, from roasted and toasted kernels (most common type) |
almond paste | Nuts, almond paste | cup, firmly packed
sunflower seeds | Seeds, sunflower seed kernels, dried | cup
pumpkin seeds, pepitas | Seeds, pumpkin and squash seed kernels, dried |
sesame seeds | Seeds, sesame seeds, whole, dried |
flaxseed, flaxseeds, flax seeds, linseed | Seeds, flaxseed | cup, whole
ground flaxseed, flaxseed meal | Seeds, flaxseed | tbsp, ground
poppy seeds | Spices, poppy seed |
hemp seeds, hemp hearts | Seeds, hemp seed, hulled |
trail mix | Snacks, trail mix, regular |

raisins | Raisins, dark, seedless (Includes foods for USDA's Food Distribution Program) | cup (not packed)
packed raisins | Raisins, dark, seedless (Includes foods for USDA's Food Distribution Program) | cup, packed
golden raisins, sultanas | Raisins, golden, seedless | cup (not packed)
currants, dried currants, zante currants | Currants, zante, dried |
dates, chopped dates | Dates, deglet noor | cup, chopped
dried cranberries, craisins | Cranberries, dried, sweetened (Includes foods for USDA's Food Distribution Program) | cup
dried blueberries | Blueberries, dried, sweetened | cup
prunes, dried plums | Plums, dried (prunes), uncooked | cup, pitted
dried apricots | Apricots, dried, sulfured, uncooked | cup, halves
dried figs | Figs, dried, uncooked |
dried apples | Apples, dried, sulfured, uncooked |
dried peaches | Peaches, dried, sulfured, uncooked | cup, halves
dehydrated apples | Apples, dehydrated (low moisture), sulfured, uncooked |
dehydrated apricots | Apricots, dehydrated (low-moisture), sulfured, uncooked |
dehydrated peaches | Peaches, dehydrated (low-moisture), sulfured, uncooked |
dehydrated prunes | Prunes, dehydrated (low-moisture), uncooked |
dehydrated carrots | Carrot, dehydrated |
sun-dried tomatoes, sun dried tomatoes | Tomatoes, sun-dried |
banana powder | Bananas, dehydrated, or banana powder |

blueberries, blueberry | Blueberries, raw |
frozen blueberries | Blueberries, frozen, unsweetened (Includes foods for USDA's Food Distribution Program) | cup, unthawed
raspberries, raspberry | Raspberries, raw |
strawberries, strawberry, whole strawberries | Strawberries, raw | cup, whole
sliced strawberries | Strawberries, raw | cup, sliced
blackberries, blackberry | Blackberries, raw |
cherries, sweet cherries, pitted cherries | Cherries, sweet, raw | cup, without pits
sour cherries | Cherries, sour, red, raw | cup, without pits
cranberries, fresh cranberries | Cranberries, raw | cup, whole
grapes, grape | Grapes, red or green (European type, such as Thompson seedless), raw |
gooseberries | Gooseberries, raw |
elderberries | Elderberries, raw |
mulberries | Mulberries, raw |
red currants | Currants, red and white, raw |
black currants, blackcurrants | Currants, european black, raw |
mango, mangoes, mangos, diced mango | Mangos, raw | cup pieces
pineapple, pineapple chunks | Pineapple, raw, all varieties | cup, chunks
apples, apple, chopped apples, chopped apple, diced apple | Apples, raw, with skin (Includes foods for USDA's Food Distribution Program) | cup, quartered or chopped
sliced apples, apple slices | Apples, raw, with skin (Includes foods for USDA's Food Distribution Program) | cup slices
bananas, banana, mashed bananas, mashed banana | Bananas, raw | cup, mashed
sliced bananas, sliced banana | Bananas, raw | cup, sliced
oranges, orange segments | Oranges, raw, all commercial varieties | cup, sections
peaches, sliced peaches | Peaches, yellow, raw | cup slices
plums | Plums, raw | cup, sliced
pears | Pears, raw | cup, slices
apricots | Apricots, raw | cup, halves
kiwi, kiwifruit | Kiwifruit, green, raw | cup, sliced
watermelon | Watermelon, raw | cup, diced
cantaloupe | Melons, cantaloupe, raw | cup, diced
avocado, avocados, diced avocado | Avocados, raw, all commercial varieties | cup, cubes
mashed avocado | Avocados, raw, all commercial varieties | cup, pureed
guavas, guava | Guavas, common, raw |
litchis, lychees, lychee | Litchis, raw |
mangosteen | Mangosteen, canned, syrup pack | cup, drained
pomegranate seeds, pomegranate arils | Pomegranates, raw | cup arils (seed/juice sacs)
applesauce, apple sauce | Applesauce, canned, unsweetened, without added ascorbic acid (Includes foods for USDA's Food Distribution Program) |
pumpkin, pumpkin puree, canned pumpkin | Pumpkin, canned, without salt |
lemon zest, lemon peel | Lemon peel, raw | tbsp
orange zest, orange peel | Orange peel, raw | tbsp

spinach | Spinach, raw |
kale | Kale, raw |
arugula, rocket | Arugula, raw | cup
lettuce, shredded lettuce | Lettuce, iceberg (includes crisphead types), raw | cup shredded
broccoli, broccoli florets | Broccoli, raw | cup chopped
cauliflower | Cauliflower, raw | cup chopped (1/2" pieces)
carrots, carrot, chopped carrots | Carrots, raw | cup chopped
grated carrots, grated carrot, shredded carrots | Carrots, raw | cup grated
sliced carrots | Carrots, raw | cup strips or slices
onions, onion, chopped onions, chopped onion, diced onion | Onions, raw | cup, chopped
sliced onions | Onions, raw | cup, sliced
garlic, minced garlic | Garlic, raw |
celery, chopped celery | Celery, raw | cup chopped
bell peppers, bell pepper, sweet peppers | Peppers, sweet, red, raw | cup, chopped
tomatoes, tomato, chopped tomatoes, diced tomatoes | Tomatoes, red, ripe, raw, year round average | cup, chopped or sliced
cherry tomatoes | Tomatoes, red, ripe, raw, year round average | cup cherry tomatoes
canned tomatoes | Tomatoes, red, ripe, canned, packed in tomato juice |
tomato sauce | Tomato products, canned, sauce |
tomato paste | Tomato products, canned, paste, without salt added (Includes foods for USDA's Food Distribution Program) |
tomato puree, passata | Tomato products, canned, puree, without salt added |
mushrooms, sliced mushrooms | Mushrooms, white, raw | cup, pieces or slices
whole mushrooms | Mushrooms, white, raw | cup, whole
corn, corn kernels, sweet corn | Corn, sweet, yellow, raw |
green beans, string beans | Beans, snap, green, raw | cup 1/2" pieces
zucchini, courgette | Squash, summer, zucchini, includes skin, raw | cup, chopped
cabbage, shredded cabbage | Cabbage, raw | cup, shredded
beets, beetroot | Beets, raw |
leeks | Leeks, (bulb and lower leaf-portion), raw |
shallots | Shallots, raw | tbsp chopped
asparagus | Asparagus, raw |
brussels sprouts | Brussels sprouts, raw |
okra | Okra, raw |
celeriac, celery root | Celeriac, raw |
eggplant, aubergine | Eggplant, raw | cup, cubes
potatoes, diced potatoes | Potatoes, flesh and skin, raw | cup, diced
mashed potatoes | Potatoes, mashed, home-prepared, whole milk and butter added |
sweet potatoes, sweet potato | Sweet potato, raw, unprepared (Includes foods for USDA's Food Distribution Program) | cup, cubes
sauerkraut | Sauerkraut, canned, solids and liquids | cup
chives | Chives, raw | tbsp chopped
parsley | Parsley, fresh | cup chopped
cilantro, coriander leaves | Coriander (cilantro) leaves, raw | cup
basil, basil leaves | Basil, fresh | cup leaves, whole
mint, mint leaves, spearmint | Spearmint, fresh | tbsp
ginger, grated ginger | Ginger root, raw | tsp
lemongrass, lemon grass | Lemon grass (citronella), raw |
watercress | Watercress, raw | cup, chopped
artichoke hearts | Artichokes, (globe or french), cooked, boiled, drained, without salt | cup hearts
alfalfa sprouts | Alfalfa seeds, sprouted, raw |
purslane | Purslane, raw |
grape leaves, vine leaves | Grape leaves, raw |
pumpkin leaves | Pumpkin leaves, raw |
amaranth leaves | Amaranth leaves, raw |
succotash | Succotash, (corn and limas), cooked, boiled, drained, without salt |
poi | Poi |

salt, table salt, fine salt | Salt, table |
baking powder | Leavening agents, baking powder, double-acting, sodium aluminum sulfate |
baking soda, bicarbonate of soda, sodium bicarbonate | Leavening agents, baking soda |
cream of tartar | Leavening agents, cream of tartar |
yeast, active dry yeast, dry yeast, instant yeast | Leavening agents, yeast, baker's, active dry |
cocoa, cocoa powder, unsweetened cocoa | Cocoa, dry powder, unsweetened |
dutch-process cocoa, dutch process cocoa | Cocoa, dry powder, unsweetened, processed with alkali |
chocolate chips, semisweet chocolate chips, chocolate morsels | Candies, semisweet chocolate | cup chips (6 oz package)
mini chocolate chips | Candies, semisweet chocolate | cup mini chips
milk chocolate chips | Candies, milk chocolate | cup chips
baking chocolate, unsweetened chocolate, grated chocolate | Baking chocolate, unsweetened, squares | cup, grated
vanilla, vanilla extract | Vanilla extract |
cinnamon, ground cinnamon | Spices, cinnamon, ground |
nutmeg, ground nutmeg | Spices, nutmeg, ground |
ground ginger | Spices, ginger, ground |
cloves, ground cloves | Spices, cloves, ground |
allspice | Spices, allspice, ground |
cardamom | Spices, cardamom | tbsp, ground
cumin, cumin seeds, ground cumin | Spices, cumin seed | tbsp, whole
coriander, coriander seeds, ground coriander | Spices, coriander seed |
paprika | Spices, paprika |
turmeric | Spices, turmeric, ground |
chili powder, chile powder | Spices, chili powder |
cayenne, cayenne pepper | Spices, pepper, red or cayenne |
pepper, black pepper, ground pepper, ground black pepper | Spices, pepper, black | tbsp, ground
white pepper | Spices, pepper, white | tbsp, ground
curry powder | Spices, curry powder |
garlic powder | Spices, garlic powder |
onion powder | Spices, onion powder |
oregano, dried oregano | Spices, oregano, dried | tsp, leaves
dried basil | Spices, basil, dried | tbsp, leaves
thyme, dried thyme | Spices, thyme, dried | tbsp, leaves
rosemary, dried rosemary | Spices, rosemary, dried |
sage | Spices, sage, ground |
dill, dill weed, dried dill | Spices, dill weed, dried |
dill seed | Spices, dill seed |
dried parsley, parsley flakes | Spices, parsley, dried |
bay leaves | Spices, bay leaf | tbsp, crumbled
caraway, caraway seeds | Spices, caraway seed |
celery seed | Spices, celery seed |
fennel seeds, fennel seed | Spices, fennel seed | tbsp, whole
fenugreek | Spices, fenugreek seed |
dry mustard, mustard powder, ground mustard | Spices, mustard seed, ground |
marjoram | Spices, marjoram, dried |
tarragon | Spices, tarragon, dried | tbsp, leaves
saffron | Spices, saffron |
mace | Spices, mace, ground |
anise seed, aniseed | Spices, anise seed | tbsp, whole
pumpkin pie spice | Spices, pumpkin pie spice |
poultry seasoning | Spices, poultry seasoning |
chervil | Spices, chervil, dried |

coffee, brewed coffee | Beverages, coffee, brewed, prepared with tap water | cup (8 fl oz)
tea, brewed tea | Beverages, tea, black, brewed, prepared with tap water | cup (8 fl oz)
apple juice | Apple juice, canned or bottled, unsweetened, with added ascorbic acid |
orange juice | Orange juice, raw (Includes foods for USDA's Food Distribution Program) |
lemon juice | Lemon juice, raw |
lime juice | Lime juice, raw |
grape juice | Grape juice, canned or bottled, unsweetened, without added ascorbic acid |
cranberry juice | Cranberry juice, unsweetened |
pineapple juice | Pineapple juice, canned or bottled, unsweetened, without added ascorbic acid |
pomegranate juice | Pomegranate juice, bottled |
tomato juice | Tomato juice, canned, without salt added |
apricot nectar | Apricot nectar, canned, with added ascorbic acid |
mango nectar | Mango nectar, canned |
vinegar, white vinegar, distilled vinegar | Vinegar, distilled |
cider vinegar, apple cider vinegar | Vinegar, cider |
red wine vinegar, wine vinegar | Vinegar, red wine |
balsamic vinegar, balsamic | Vinegar, balsamic |
soy sauce | Soy sauce made from soy and wheat (shoyu) |
fish sauce | Sauce, fish, ready-to-serve |
oyster sauce | Sauce, oyster, ready-to-serve |
worcestershire sauce, worcestershire | Sauce, worcestershire |
ketchup, catsup, tomato ketchup | Catsup |
mustard, yellow mustard, prepared mustard | Mustard, prepared, yellow |
mayonnaise, mayo, mayonnaise dressing | Salad dressing, mayonnaise, regular |
sriracha | Sauce, hot chile, sriracha |
hot sauce | Sauce, ready-to-serve, pepper, TABASCO |
barbecue sauce, bbq sauce | Sauce, barbecue |
salsa | Sauce, salsa, ready-to-serve |
pesto | Sauce, pesto, ready-to-serve, refrigerated |
jam, preserves | Jams and preserves |
marmalade | Marmalade, orange |
broth, stock, chicken broth, chicken stock | Soup, stock, chicken, home-prepared |
beef broth, beef stock | Soup, stock, beef, home-prepared |
fish stock | Soup, stock, fish, home-prepared |
vegetable broth, vegetable stock | Soup, vegetable broth, ready to serve |
wine | Alcoholic beverage, wine, table, all |
red wine | Alcoholic beverage, wine, table, red |
white wine | Alcoholic beverage, wine, table, white |
beer | Alcoholic beverage, beer, regular, all |
vodka, rum, gin, whiskey, whisky | Alcoholic beverage, distilled, all (gin, rum, vodka, whiskey) 80 proof |
"""


def measure_unit(modifier):
    """The volume unit a household measure starts with: "cup, packed" is a cup."""
    for unit in sorted(PER_CUP, key=len, reverse=True):
        if modifier == unit or modifier.startswith((unit + " ", unit + ",")):
            return unit
    return None


def grams_per_cup(portions, measure):
    """Grams in a cup from the measure asked for, or from the plain cup, tablespoon..."""
    wanted = [measure] if measure else PLAIN
    for name in wanted:
        for amount, modifier, grams in portions:
            unit = measure_unit(modifier)
            if modifier == name and unit and amount > 0:
                return grams / amount * PER_CUP[unit]
    return None


def number(d):
    """Exact if it ends within 7 decimals, else to two: 125, 227.2, 236.5882365, 206.06."""
    if d != d.quantize(Decimal("1e-7")):
        d = d.quantize(Decimal("0.01"))
    return format(d.normalize(), "f")


def main(folder):
    descriptions = {r["fdc_id"]: r["description"] for r in csv.DictReader(open(f"{folder}/food.csv"))}
    portions = {}
    for r in csv.DictReader(open(f"{folder}/food_portion.csv")):
        amount = Decimal(r["amount"] or "0")
        portions.setdefault(descriptions[r["fdc_id"]], []).append(
            (amount, r["modifier"].strip(), Decimal(r["gram_weight"]))
        )
    rows = {}

    def add(names, grams):
        for name in names.split(","):
            name = name.strip()
            if name in rows:
                sys.exit(f"{name}: listed twice")
            rows[name] = grams

    for names, grams in FIXED.items():
        add(names, grams)
    for line in FOODS.strip().splitlines():
        if not line.strip():
            continue
        names, description, measure = (part.strip() for part in line.split("|"))
        if description not in portions:
            sys.exit(f"{description}: not in SR Legacy")
        grams = grams_per_cup(portions[description], measure)
        if grams is None:
            known = "; ".join(m for _, m, _ in portions[description])
            sys.exit(f"{description}: no measure {measure or 'cup'} (has {known})")
        add(names, grams)

    print("# Grams in a US cup (236.5882365 mL) of foods, for weight to volume and back.")
    print("# From USDA FoodData Central SR Legacy (public domain); built by scripts/foods.py.")
    for name in sorted(rows):
        print(f"{name} = {number(rows[name])}")


if __name__ == "__main__":
    main(sys.argv[1])
