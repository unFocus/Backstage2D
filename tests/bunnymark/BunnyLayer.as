/*
========================================================================
=      ================================  =====  ==================  ====
=  ===  ===============================   ===   ==================  ====
=  ====  ==============================  =   =  ==================  ====
=  ===  ===  =  ==  = ===  = ===  =  ==  == ==  ===   ===  =   ===  =  =
=      ====  =  ==     ==     ==  =  ==  =====  ==  =  ==    =  ==    ==
=  ===  ===  =  ==  =  ==  =  ===    ==  =====  =====  ==  =======   ===
=  ====  ==  =  ==  =  ==  =  =====  ==  =====  ===    ==  =======    ==
=  ===  ===  =  ==  =  ==  =  ==  =  ==  =====  ==  =  ==  =======  =  =
=      =====    ==  =  ==  =  ===   ===  =====  ===    ==  =======  =  =
========================================================================


* Copyright (c) 2012 Julian Wixson / Aaron Charbonneau - Adobe Systems
*
* Special thanks to Iain Lobb - iainlobb@googlemail.com for the original BunnyMark:
*
* http://blog.iainlobb.com/2010/11/display-list-vs-blitting-results.html 
*
* This program is distributed under the terms of the MIT License as found 
* in a file called LICENSE. If it is not present, the license
* is always available at http://www.opensource.org/licenses/mit-license.php.
*
* This program is distributed in the hope that it will be useful, but
* without any waranty; without even the implied warranty of merchantability
* or fitness for a particular purpose. See the MIT License for full details.
*/

package bunnymark
{
	import backstage2d.display.Image;
	import backstage2d.display.Layer;
	import flash.display.Bitmap;
	import flash.display.BitmapData;
	import flash.display3D.*;
	import flash.geom.Point;
	import flash.geom.Rectangle;
	
	public class BunnyLayer
	{
		[Embed(source="../../assets/wabbit_alpha.png")]
		private var BunnyImage : Class;
		private var bunnyBMD:BitmapData;
		private var _bunnies : Vector.<BunnySprite>;
		
		private var layer:Layer;
		
		private var gravity:Number = 0.5;
		private var maxX:int;
		private var minX:int;
		private var maxY:int;
		private var minY:int;	
		
		public function BunnyLayer(view:Rectangle)
		{
			setPosition(view);
			_bunnies = new Vector.<BunnySprite>();
			
		}
		public function setPosition(view:Rectangle):void {
			maxX = view.width;
			minX = view.x;
			maxY = view.height;
			minY = view.y;
		}
		
		public function createRenderLayer() : Layer
		{
			// make the bunny bitmapData, and store a reference
			bunnyBMD = (new BunnyImage() as Bitmap).bitmapData
			
			// Create new render layer 
			layer = new Layer();
			
			return layer;
		}
		
		public function addBunny(numBunnies:int):void
		{
			var currentBunnyCount:int = _bunnies.length;
			
			var bunny:BunnySprite;
			var sprite:Image;
			
			for ( var i:uint = currentBunnyCount ; i < currentBunnyCount + numBunnies; i++ )
			{
				sprite = new Image( bunnyBMD );
				bunny = new BunnySprite(sprite);
				
				//bunny.sprite.position = new Point();
				bunny.speedX = Math.random() * 5;
				bunny.speedY = (Math.random() * 5) - 2.5;
				bunny.sprite.scaleX = 0.3 + Math.random();
				bunny.sprite.scaleY = bunny.sprite.scaleX;
				bunny.sprite.rotation = 15 - Math.random() * 30;
				_bunnies.push(bunny);
				layer.add( bunny.sprite );
			}
		}
		
		public function update(currentTime:Number) : void
		{		
			var bunny:BunnySprite;
			for(var i:int=0; i<_bunnies.length;i++)
			{
				bunny = _bunnies[i];
				bunny.sprite.x += bunny.speedX;
				bunny.sprite.y += bunny.speedY;
				bunny.speedY += gravity;
				bunny.sprite.opacity = 0.3 + 0.7 * bunny.sprite.y / maxY;
				
				if (bunny.sprite.x > maxX)
				{
					bunny.speedX *= -1;
					bunny.sprite.x = maxX;
				}
				else if (bunny.sprite.x < minX)
				{
					bunny.speedX *= -1;
					bunny.sprite.x = minX;
				}
				if (bunny.sprite.y > maxY)
				{
					bunny.speedY *= -0.8;
					bunny.sprite.y = maxY;
					if (Math.random() > 0.5) bunny.speedY -= 3 + Math.random() * 4;
				} 
				else if (bunny.sprite.y < minY)
				{
					bunny.speedY = 0;
					bunny.sprite.y = minY;
				}	
			}
		}
	}
}