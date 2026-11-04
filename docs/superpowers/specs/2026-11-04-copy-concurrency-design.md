# Souběh budoucího kopírování podle úložiště

Datum: 4. listopadu 2026

Stav: **návrh, bez implementace**. Copy/Move/Delete zůstávají vypnuté. Tento
doplněk zpřesňuje oddíly 8, 9, 17 a 25 hlavního návrhu; nemění `SECURITY.md` ani
povinné ověření každého souboru. Číselné výchozí limity jsou doporučení návrhu,
která se musí ověřit měřením, nikoli tvrzení o naměřeném výkonu aplikace.

## Proč omezený souběh

Ano, paralelní kopírování několika souborů může pomoci využít lokální SSD nebo
překrýt latence malých souborů. Microsoft nabízí Robocopy `/MT`; dokumentace SMB
popisuje překrytí vytváření souborů a přenosu a varuje před příliš mnoha vlákny.
To nedokazuje, že více vláken zrychlí každou lokální SSD kopii nebo jediný velký
soubor. [Robocopy](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/robocopy),
[Microsoft: small-file transfers](https://learn.microsoft.com/en-us/troubleshoot/windows-server/networking/slow-smb-file-transfer#slow-transfer-when-using-small-files)

Výrobce Kingston rozlišuje queue depth, propustnost a latenci a doporučuje dlouhé
testy se skutečným mixem čtení/zápisu. Proto se měří celá ověřená kopie, nikoli
pouze krátký zápis do cache. [Kingston: SSD test metrics](https://www.kingston.com/en/blog/servers-and-data-centers/how-to-test-enterprise-ssd-part-2)
Solidigm obdobně odlišuje vysokou benchmarkovou queue depth od nízké queue depth
svých měřených desktopových úloh. Z toho plyne požadavek našeho návrhu měřit
propustnost společně s latencí; žádný z těchto zdrojů neurčuje univerzální počet
kopírovacích workerů. [Solidigm: PC Storage Performance in the Real World](https://www.solidigmtechnology.com/content/dam/solidigm/en/site/products/technology/performance-with-impact/documents/Performance-With-Impact-White-Paper-R2.pdf)

| Přístup | Přínos a náklad | Rozhodnutí návrhu |
| --- | --- | --- |
| Jeden proud všude | Jednoduchý a konzervativní; nemusí využít SSD a latence malých souborů | Výchozí pro HDD/síť/neznámé |
| Omezený souběh různých souborů | Překrývá nezávislou práci; vyžaduje sdílené rozpočty a férovost | První SSD implementace |
| Paralelní bloky jediného souboru | Možný přínos ve specifických podmínkách; složitější I/O, hashování a rušení | Odloženo do samostatně měřené etapy |

## Rozpočty a fyzická identita

- Pro pozitivně určené **lokální SSD** navrhnout 2 aktivní I/O proudy na zařízení.
  Nastavení smí snížit limit na 1; zvýšení nejvýše na 4 vyžaduje prokázaný přínos
  bez zhoršení interaktivní odezvy. Počet CPU jader neurčuje tento limit.
- HDD má 1 proud na zařízení. Síť má 1 ve sdílené skupině endpointu; nelze-li
  prokázat nezávislost endpointů/aliasů, sdílí konzervativní síťový rozpočet 1.
  Neznámá, smíšená či nejednoznačná topologie sdílí konzervativní rozpočet 1;
  písmeno disku ani rychlost připojení nejsou důkaz lokálního SSD.
- Backendový adaptér vrací konzervativní fyzické identity a jejich rozsah, nikoli
  samotný řetězec mountpointu. Dva oddíly jednoho disku, bind mounty, hardlinkové
  a jiné zjistitelné aliasy nesmějí násobit kapacitu. RAID/Storage Spaces/virtuální
  zařízení vyžadují prokazatelnou topologii nebo konzervativní skupinu. Chyba
  detekce nepovolí vyšší limit. Síťový server s SSD stále používá síťovou politiku.
- Zdrojové čtení, cílový zápis, sync i **úplný ověřovací readback** spotřebovávají
  kapacitu příslušného úložiště. Zdroj i cíl na stejném fyzickém SSD používají
  společný rozpočet. První verze zpracovává bloky každého souboru sekvenčně;
  jeden permit lze použít pro jeho střídající se čtení/zápis/readback na témže
  zařízení. Pokud se fáze překrývají, každý skutečně souběžný I/O proud musí mít
  permit. Dva aktivní soubory nesmějí skrytě vytvořit čtyři neúčtované proudy.
- Různá prokázaná fyzická zařízení mohou pracovat současně: například čtení
  dalšího aktivního souboru na SSD A se překryje se zápisem jiného na SSD B nebo
  s jeho ověřením. Každá fáze má vlastní účtování zařízení a omezený buffer.
  To umožňuje pipeline mezi soubory bez paralelního čtení bloků jednoho souboru.
  Násobení kopií nad dvěma písmeny téhož disku takovou nezávislost nezakládá.
- Přijetí práce potřebující více zařízení provede scheduler najednou, nebo
  zkusí získat seřazené identity a při neúspěchu všechny dílčí permity vrátí
  před čekáním; blokující worker nečeká s neúplnou rezervací. Nevznikne
  deadlock kopiemi A→B a B→A. Identita se kontroluje při otevření; změněná či
  neověřitelná topologie vede ke konzervativnímu přeplánování před dalším I/O.
- Globální strop pracovníků, počet otevřených souborů, CPU hashování, počet
  připravených položek a celkové bajty bufferů mají vlastní limity. Nedostatek
  prostředků vyvolá backpressure nebo konkrétní chybu; nezkracuje nativní cesty.

Tyto hodnoty omezují aplikační I/O proudy, nikoli skutečnou hardwarovou queue
depth řízenou také OS a zařízením. Není to požadavek na dvě nová OS vlákna pro
každý disk nebo soubor. Tokio vlastní omezené logické úlohy a plánování;
blokující systémová volání a hashování běží pouze na backendu, nikdy v UI ani
v dlouhém CPU úseku běžného async tasku. Použití `spawn_blocking` samo nelimituje
aplikační souběh: permit se získá před spuštěním; pro dlouho žijící pracovní
smyčky lze zvolit pevný dedikovaný pool. [Tokio: spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)

## Férovost, fronta a zrušení

Jedna operace obsahuje postupně objevované souborové podúlohy. Zpočátku smí být
aktivní jedna mutující operace, ale její nezávislé soubory mohou sdílet SSD
kapacitu. Přijetí dalších Copy/Move/Delete nesmí čekat na její dokončení.
Budoucí souběh nezávislých operací používá tytéž rozpočty a zámky cest/podstromů
i zjistitelných aliasů; nezískává nový limit pro každou frontu či panel.

Plánování používá omezené okno kandidátů a střídání souborových podúloh. Po
omezeném úseku zpracovaných bajtů/času a dokončení probíhajícího I/O vrátí velký
soubor pracovní permit do plánování, pokud čeká jiná připravená práce. Zachová
handle, offset, hash a vlastnictví dočasného cíle; tento pozastavený stav se také
počítá do limitu otevřených souborů/paměti. Malé soubory aktivní operace tak
dostanou příležitost, i když všechny sloty původně obsadily velké soubory.
Stárnutí čekající práce zabrání opačnému vyhladovění velkého souboru. Konkrétní
kvantum se zvolí měřením; na HDD musí omezit zbytečné seeky a zachovat jediný
sekvenční proud. Nikdy se násilně nepřeruší běžící OS volání.

Bufferové permity se přidělují po omezených blocích, ne podle celé délky souboru;
velká čekající byte-permit žádost nesmí zablokovat všechny menší. Neexistuje
neomezené přednačítání nebo task/vlákno na každý soubor. Tato férovost platí
uvnitř aktivní operace; v prvním FIFO milníku nová malá operace nepředbíhá celou
starší mutující operaci. Odezva přijetí do fronty a doba do skutečného spuštění
jsou odlišné metriky.

Interaktivní navigace má oddělenou pracovní kapacitu a přednost při příštím
přidělení sdíleného zařízení; nepředbíhá násilně běžící OS volání. Před zapnutím
kopírování musí integrace background scanů s kopírováním společně respektovat
zařízení a prioritu, včetně navigace a ověřovacího čtení. Není přípustné sečíst
nezávislé limity skenu a kopie pro stejné zařízení.
Stávající globální čtyřslotový vážený scheduler velikostí složek zůstává nyní
beze změny; nelze jej pouze zkopírovat, protože kopie zahrnuje zápis, dvě zařízení,
verifikaci, zámky a publikování. V této etapě nevzniká nový filesystem service.

Rušení je kooperativní mezi bloky a fázemi. Už spuštěná blokující operace drží
permit, handle, buffer i vlastnictví dočasného cíle do skutečného návratu.
Timeout/abort nejsou důkaz dokončení a neuvolní skrytě další pracovní kapacitu.
Úklid čeká na všechny inflight operace. Čekání na konflikt neblokuje pracovní
vlákno ani nové požadavky; pokračovat mohou jen soubory bez závislosti na tomto
rozhodnutí. [Tokio: cancellation of blocking work](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)

## Integrita každého souboru

Každý soubor má vlastní otevřenou zdrojovou identitu, bezpečně výhradně vytvořený
nepředvídatelný dočasný cíl vedle finálního cíle, BLAKE3-256 stav a průběh fází.
Hashuje se přesný proud přečtený ze zdroje ve správném pořadí. Po dokončení zápisů,
slíbených metadat a povinné synchronizace následuje **samostatné úplné čtení
dočasného cíle** a porovnání délky i hashe. Hash outgoing bufferu podruhé se
nepočítá. Chyba nebo zjištěná změna zdroje zabrání úspěchu/publikování. Kontroly
metadat nejsou atomický snapshot proti souběžným zapisovačům; cached readback
není důkaz fyzického přečtení média ani absolutní odolnosti proti výpadku napájení.

Publikování je samostatný krok každého ověřeného souboru. No-replace musí atomicky
odmítnout souběžně vzniklý cíl; existující cíl se nikdy předem nemaže. Přepsání
vyžaduje platné rozhodnutí s velikostí/časem/identitou a deklarovaný providerový
kontrakt z oddílu 25 hlavního návrhu, včetně závodu po poslední revalidaci.
Nepodporovaný kontrakt znamená Unsupported, ne slabší tichý fallback. Celý strom
není transakce; již publikované soubory zůstávají při chybě nebo zrušení dalších.
Destruktivní závislost smí pokračovat až po úplném ověřeném Completed kopie,
nikoli po zápisu posledního bajtu nebo částečném dokončení.

Pro první verzi postačí inkrementální CPU `Hasher::update`. Případný paralelní
pipeline jednoho velkého souboru musí odděleně řešit offsety, krátké I/O,
omezené přeuspořádání a BLAKE3 stromovou konstrukci; nelze hashovat bloky v pořadí
dokončení ani slepit jejich digests. Oficiální `update_rayon` je volitelný a může
být pro malé vstupy pomalejší; nezavádí se bez end-to-end měření a CPU rozpočtu.
Načíst celý soubor do RAM kvůli němu není řešení. [BLAKE3: Hasher](https://docs.rs/blake3/latest/blake3/struct.Hasher.html#method.update_rayon)
GPU zůstává pro vykreslování; GPU hashování se nepoužije, dokud měření neprokáže
přínos celé ověřené kopii včetně přenosů přes PCIe a synchronizace.

## Přejímací měření a bezpečnostní testy

Porovnat 1/2/4 proudy se stejnými daty a povinným readbackem: mnoho malých,
smíšené velikosti, jeden velký a více velkých souborů; jedno SSD se dvěma oddíly,
dvě fyzická SSD, HDD, SSD↔HDD, síť a neznámou topologii. Testovat cold/warm cache,
dlouhé běhy nad krátkou cache, rozdílné směry čtení/zápisu a současnou navigaci.
Uvést hardware/filesystem, celkový čas do ověřeného publikování, files/s,
propustnost, CPU, maximální buffer/paměť, skutečný souběh a p95/p99 odezvy UI.
Odděleně zaznamenat p95/p99 čekání malého souboru, dobu potvrzení zařazení další
Copy/Move/Delete, dobu čekání této operace ve FIFO a odezvu žádosti o zrušení
oproti skutečnému doběhu. Porovnat i samotnou navigaci bez kopie; cíle hlavního
návrhu jsou měřené akceptační cíle, nikoli již dosažené výsledky.
Vyšší limit zvolit jen při opakovatelném přínosu bez zhoršení bezpečnosti/odezvy;
jediný velký soubor neslibuje škálování podle počtu vláken.

Testy musí vložit poškození a truncation cíle, stejně velkou editaci, změnu zdroje,
short read/write, read-only/oprávnění, disk-full/disconnect, chyby sync/publish,
kolizi při publikování i aliasy zdroj=dest. Zrušit v každé fázi a prokázat, že
permity se uvolní až po skutečném doběhu a původní cíl zůstane zachován před
publikováním. Ověřit dvě fronty nad stejným SSD, opačné kopie A↔B, malé soubory
vedle velkých, zaplnění front a bufferového rozpočtu i čekání na overwrite dialog.
Unicode/raw nativní názvy a dynamicky dlouhé cesty nesmějí změnit identitu nebo
zavést aplikační MAX_PATH limit. Všechny tyto testy patří k budoucí implementaci;
nejsou tvrzením, že kopírování už existuje nebo bylo otestováno.
